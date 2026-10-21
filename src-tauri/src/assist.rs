//! Visible Nexus download-assist WebView (free-account path).

use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
use tauri::Rect;
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager, Webview, WebviewUrl};

use crate::commands::AppState;

pub const ASSIST_LABEL: &str = "nexus-download-assist";
pub const MAIN_WINDOW_LABEL: &str = "main";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AssistBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for AssistBounds {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 480.0,
            height: 360.0,
        }
    }
}

const NXM_SENTINEL_HOST: &str = "emperor-mod-manager.invalid";
const NXM_SENTINEL_PATH: &str = "/capture-nxm";
const CDN_SENTINEL_PATH: &str = "/capture-download";

/// Two-phase free download autoclick: open Free/Premium dialog, then click Slow download.
const AUTOCLICK_JS: &str = r#"
(function () {
  var key = (location.href || '').split('#')[0];
  if (window.__emperorModManagerAutoclickKey === key) return;
  window.__emperorModManagerAutoclickKey = key;
  if (window.__emperorModManagerAutoclickTimer) {
    clearInterval(window.__emperorModManagerAutoclickTimer);
    window.__emperorModManagerAutoclickTimer = null;
  }
  var started = Date.now();
  var deadline = started + 180000;
  var retryCooldownMs = 800;
  var lastClickAt = 0;
  var pageKeyAtStart = key;
  var done = false;
  var assist = window.__emperorModManagerAssist || {};
  var targetFileId = (assist.fileId || '') + '';
  var targetModId = (assist.modId || '') + '';
  var state = window.__emperorModManagerAutoclickState || {};
  window.__emperorModManagerAutoclickState = state;
  state.installed = true;
  state.pageKey = key;
  state.phase = 'openDialog';
  state.polls = 0;
  state.candidates = 0;
  state.slowFound = false;
  state.slowLabel = '';
  state.slowTag = '';
  state.downloadLabels = [];
  state.clickableHits = 0;
  state.nxmButtonCount = 0;
  state.bodyHasSlow = false;
  state.bodyHasManual = false;
  state.iframeCount = 0;
  state.openShadowHosts = 0;
  state.deepNxmWeakCount = 0;
  state.deepHasSlow = false;
  state.lastAction = '';
  state.lastAt = 0;

  function touch(action) {
    state.lastAction = action;
    state.lastAt = Date.now();
  }

  function label(el) {
    return ((el.innerText || el.textContent || el.value || el.getAttribute('aria-label') || '') + '')
      .toLowerCase()
      .replace(/\s+/g, ' ')
      .trim();
  }

  function bodyText() {
    return ((document.body && document.body.innerText) || '').toLowerCase();
  }

  function enabled(el) {
    if (!el) return false;
    if (el.disabled) return false;
    if (el.getAttribute('aria-disabled') === 'true') return false;
    if (el.classList && (el.classList.contains('disabled') || el.classList.contains('is-disabled'))) return false;
    return true;
  }

  function classString(el) {
    if (!el || !el.className) return '';
    if (typeof el.className === 'string') return el.className;
    if (el.className.baseVal) return el.className.baseVal + '';
    return el.className + '';
  }

  function visuallyHidden(node) {
    try {
      var style = window.getComputedStyle(node);
      if (style.display === 'none' || style.visibility === 'hidden' || parseFloat(style.opacity || '1') === 0) return true;
      var overflow = (style.overflow || '') + (style.overflowX || '') + (style.overflowY || '');
      if (overflow.indexOf('hidden') !== -1) {
        var box = node.getBoundingClientRect();
        if (box.width < 4 || box.height < 4) return true;
      }
    } catch (e) {
      return true;
    }
    return false;
  }

  function rendered(el) {
    if (!enabled(el)) return false;
    try {
      if (visuallyHidden(el)) return false;
      var style = window.getComputedStyle(el);
      var rect = el.getBoundingClientRect();
      if (rect.width < 4 || rect.height < 4) return false;
      if (el.offsetParent === null && style.position !== 'fixed' && style.position !== 'sticky') {
        var rootNode = el.getRootNode && el.getRootNode();
        var inShadow = rootNode && rootNode !== document && rootNode.host;
        if (!inShadow && (!document.body || !document.body.contains(el))) return false;
      }
      var node = el;
      var guard = 0;
      while (node && guard < 40) {
        guard += 1;
        if (node !== el && visuallyHidden(node)) return false;
        if (node === document.body || node === document.documentElement) break;
        if (node.parentElement) {
          node = node.parentElement;
          continue;
        }
        var root = node.getRootNode && node.getRootNode();
        if (root && root.host && root.host !== node) {
          node = root.host;
          continue;
        }
        break;
      }
    } catch (e) {
      return false;
    }
    return true;
  }

  function walkRoots(root, visit) {
    if (!root) return;
    visit(root);
    var all;
    try {
      all = root.querySelectorAll ? root.querySelectorAll('*') : [];
    } catch (e) {
      return;
    }
    for (var i = 0; i < all.length; i++) {
      var el = all[i];
      if (el.shadowRoot) {
        state.openShadowHosts = (state.openShadowHosts || 0) + 1;
        walkRoots(el.shadowRoot, visit);
      }
      var tag = (el.tagName || '').toUpperCase();
      if (tag === 'IFRAME' || tag === 'FRAME') {
        try {
          var doc = el.contentDocument;
          if (doc) walkRoots(doc, visit);
        } catch (e) {}
      }
    }
  }

  function queryDeep(selector) {
    var out = [];
    walkRoots(document, function (root) {
      try {
        var nodes = root.querySelectorAll(selector);
        for (var i = 0; i < nodes.length; i++) out.push(nodes[i]);
      } catch (e) {}
    });
    return out;
  }

  function countIframesDeep() {
    var n = 0;
    walkRoots(document, function (root) {
      try {
        n += root.querySelectorAll('iframe, frame').length;
      } catch (e) {}
    });
    return n;
  }

  function bodyTextDeep() {
    var parts = [];
    walkRoots(document, function (root) {
      try {
        if (root.body && root.body.innerText) parts.push(root.body.innerText);
        else if (root !== document && root.innerText) parts.push(root.innerText);
      } catch (e) {}
    });
    return parts.join(' ').toLowerCase();
  }

  function hasSlowDownloadText(t) {
    if (!t) return false;
    return /\bslow\s*download\b/i.test(t);
  }

  function isCardHeading(t) {
    t = (t || '').toLowerCase();
    return t.indexOf('wait more') !== -1 && t.indexOf('slow download') !== -1;
  }

  function isClickableAncestor(el) {
    if (!el || !rendered(el)) return false;
    var tag = (el.tagName || '').toUpperCase();
    if (tag === 'BUTTON' || tag === 'A' || tag === 'INPUT') return true;
    if (el.getAttribute('role') === 'button') return true;
    try {
      if (window.getComputedStyle(el).cursor === 'pointer') return true;
    } catch (e) {}
    var tab = el.getAttribute('tabindex');
    if (tab !== null && tab !== '-1') return true;
    if (typeof el.onclick === 'function') return true;
    return false;
  }

  function climbClickable(leaf) {
    var el = leaf;
    var hits = 0;
    while (el && el !== document.body && el !== document.documentElement) {
      if (isClickableAncestor(el)) {
        hits += 1;
        return { el: el, hits: hits };
      }
      el = el.parentElement;
    }
    return { el: null, hits: hits };
  }

  function isModManager(t) {
    if (t.indexOf('slow download') !== -1) return false;
    if (t.indexOf('fast download') !== -1) return false;
    if (t.indexOf('mod manager') !== -1 && t.indexOf('download') !== -1) return true;
    if (t.indexOf('download with manager') !== -1) return true;
    if (t.indexOf('download with mod manager') !== -1) return true;
    return false;
  }

  function isManualDownload(t) {
    return t === 'manual download' || t.indexOf('manual download') === 0;
  }

  function isShortDownload(t) {
    if (!t) return false;
    if (t === 'download') return true;
    if (/^download[.!]?$/.test(t)) return true;
    return false;
  }

  function isFastOrPremium(t) {
    return t.indexOf('fast download') !== -1 ||
      t.indexOf('go premium') !== -1 ||
      t.indexOf('get premium') !== -1;
  }

  function hrefOf(el) {
    return ((el && (el.href || (el.getAttribute && el.getAttribute('href')))) || '') + '';
  }

  function isBlockedHref(href) {
    var lower = ((href || '') + '').toLowerCase();
    if (lower.indexOf('tab=download+history') !== -1) return true;
    if (lower.indexOf('tab=download%20history') !== -1) return true;
    if (lower.indexOf('tab=download history') !== -1) return true;
    if (lower.indexOf('download-history') !== -1) return true;
    if (lower.indexOf('next.nexusmods.com/vortex') !== -1) return true;
    return false;
  }

  function isHistoryUrl(href) {
    var lower = ((href || '') + '').toLowerCase();
    if (lower.indexOf('tab=download+history') !== -1) return true;
    if (lower.indexOf('tab=download%20history') !== -1) return true;
    if (lower.indexOf('tab=download history') !== -1) return true;
    if (lower.indexOf('download-history') !== -1) return true;
    return false;
  }

  function isBlockedLabel(t) {
    t = (t || '').toLowerCase();
    if (t.indexOf('download history') !== -1) return true;
    if (t.indexOf('download app') !== -1) return true;
    return false;
  }

  function isAccountOrMarketing(el) {
    if (!el) return false;
    if (isBlockedLabel(label(el))) return true;
    if (isBlockedHref(hrefOf(el))) return true;
    return false;
  }

  function nextNodeUp(node) {
    if (!node) return null;
    if (node.parentElement) return node.parentElement;
    var root = node.getRootNode && node.getRootNode();
    if (root && root.host && root.host !== node) return root.host;
    return null;
  }

  function inSiteChrome(el) {
    var node = el;
    var guard = 0;
    while (node && guard < 40) {
      guard += 1;
      var tag = (node.tagName || '').toUpperCase();
      if (tag === 'HEADER' || tag === 'NAV') return true;
      var cls = classString(node);
      if (cls.indexOf('rj-profile-tray') !== -1 || cls.indexOf('new-head') !== -1) return true;
      node = nextNodeUp(node);
    }
    return false;
  }

  function inDownloadSurface(el) {
    if (!el || inSiteChrome(el)) return false;
    if (el.classList && el.classList.contains('nxm-button')) return true;
    if ((el.id || '') === 'slowDownloadButton') return true;
    var node = el;
    var guard = 0;
    while (node && node !== document.body && node !== document.documentElement && guard < 40) {
      guard += 1;
      var tag = (node.tagName || '').toUpperCase();
      if (tag === 'HEADER' || tag === 'NAV') return false;
      if (tag === 'MOD-DOWNLOAD-MODAL' || tag === 'DIALOG') return true;
      if ((node.id || '') === 'slowDownloadButton') return true;
      var cls = classString(node).toLowerCase();
      if (cls.indexOf('nxm-modal') !== -1 || cls.indexOf('mod-download') !== -1 ||
          cls.indexOf('file-expander') !== -1 || cls.indexOf('files-tabs') !== -1 ||
          cls.indexOf('mod-file') !== -1) return true;
      var root = node.getRootNode && node.getRootNode();
      if (root && root.host && (root.host.tagName || '').toUpperCase() === 'MOD-DOWNLOAD-MODAL') return true;
      node = nextNodeUp(node);
    }
    return false;
  }

  function isDownloadStartingPage() {
    var body = bodyTextDeep();
    return body.indexOf('your download is starting') !== -1 ||
      body.indexOf('download didn\'t start') !== -1 ||
      body.indexOf('start download manually') !== -1;
  }

  function succeeded() {
    var nowKey = (location.href || '').split('#')[0];
    if (nowKey !== pageKeyAtStart && !isHistoryUrl(nowKey)) return true;
    if (isDownloadStartingPage()) return true;
    return false;
  }

  function nxmHref(el) {
    var href = ((el && (el.href || el.getAttribute && el.getAttribute('href'))) || '') + '';
    return href.indexOf('nxm://') === 0 ? href : '';
  }

  function bridgeNxm(href) {
    if (typeof window.__emperorModManagerBridgeNxm === 'function') {
      window.__emperorModManagerBridgeNxm(href);
      return;
    }
    window.location.href = 'https://emperor-mod-manager.invalid/capture-nxm?url=' + encodeURIComponent(href);
  }

  function isCdnHref(href) {
    href = (href || '') + '';
    if (href.indexOf('http') !== 0) return false;
    try {
      var u = new URL(href);
      var host = (u.hostname || '').toLowerCase();
      if (host.indexOf('nexus-cdn') !== -1) return true;
      if (host === 'cf-files.nexusmods.com') return true;
      if (host.indexOf('nexusmods.com') !== -1 && u.pathname.indexOf('/cdn/') !== -1) return true;
    } catch (e) {}
    return false;
  }

  function bridgeCdn(href) {
    if (typeof window.__emperorModManagerBridgeCdn === 'function') {
      window.__emperorModManagerBridgeCdn(href);
      return;
    }
    window.location.href = 'https://emperor-mod-manager.invalid/capture-download?url=' + encodeURIComponent(href);
  }

  function realisticClick(el) {
    try {
      el.scrollIntoView({ block: 'center', inline: 'center' });
    } catch (e) {}
    try {
      var rect = el.getBoundingClientRect();
      var x = rect.left + rect.width / 2;
      var y = rect.top + rect.height / 2;
      var opts = { bubbles: true, cancelable: true, view: window, clientX: x, clientY: y };
      if (typeof PointerEvent === 'function') {
        el.dispatchEvent(new PointerEvent('pointerdown', opts));
        el.dispatchEvent(new PointerEvent('pointerup', opts));
      }
      el.dispatchEvent(new MouseEvent('mousedown', opts));
      el.dispatchEvent(new MouseEvent('mouseup', opts));
      el.dispatchEvent(new MouseEvent('click', opts));
    } catch (e) {
      try { el.click(); } catch (e2) {}
    }
  }

  function candidates() {
    return queryDeep('a, button, input[type=button], input[type=submit], [role=button]').filter(rendered);
  }

  function collectDownloadLabels() {
    var out = [];
    var nodes = queryDeep('a, button, [role=button], input[type=button], input[type=submit]');
    for (var i = 0; i < nodes.length && out.length < 8; i++) {
      if (!rendered(nodes[i])) continue;
      var t = label(nodes[i]);
      if (t.indexOf('download') !== -1) out.push(t.slice(0, 60));
    }
    return out;
  }

  function matchesNxmSlowButton(el) {
    if (!el || !rendered(el) || !inDownloadSurface(el) || isAccountOrMarketing(el)) return false;
    var t = label(el);
    if (!hasSlowDownloadText(t) || isCardHeading(t) || isBlockedLabel(t)) return false;
    return t === 'slow download' || t.length < 40;
  }

  function findSlowDownload() {
    state.openShadowHosts = 0;
    var primary = queryDeep('button.nxm-button.nxm-button-secondary-filled-weak');
    state.nxmButtonCount = document.querySelectorAll('button.nxm-button.nxm-button-secondary-filled-weak').length;
    state.deepNxmWeakCount = primary.length;
    state.deepHasSlow = false;
    for (var i = 0; i < primary.length; i++) {
      if (matchesNxmSlowButton(primary[i])) {
        state.deepHasSlow = true;
        return primary[i];
      }
    }
    var fallback = queryDeep('button.nxm-button');
    for (var j = 0; j < fallback.length; j++) {
      if (matchesNxmSlowButton(fallback[j])) {
        state.deepHasSlow = true;
        return fallback[j];
      }
    }
    var best = null;
    var bestScore = Infinity;
    var totalHits = 0;
    var all = queryDeep('*');
    for (var k = 0; k < all.length; k++) {
      var el = all[k];
      if (!rendered(el)) continue;
      var full = label(el);
      if (!hasSlowDownloadText(full) || isCardHeading(full)) continue;
      state.deepHasSlow = true;
      var climbed = climbClickable(el);
      totalHits += climbed.hits;
      var target = climbed.el || (isClickableAncestor(el) ? el : null);
      if (!target || !inDownloadSurface(target) || isAccountOrMarketing(target)) continue;
      var score = full === 'slow download' ? 0 : full.length;
      if (score < bestScore) {
        bestScore = score;
        best = target;
      }
    }
    state.clickableHits = totalHits;
    return best;
  }

  function markupHasFileIdToken(text) {
    if (!targetFileId || text === undefined || text === null) return false;
    var hay = (text + '').toLowerCase();
    var id = (targetFileId + '').toLowerCase();
    var keys = [
      'file_id=' + id,
      'file_id%3d' + id,
      'fid=' + id,
      'data-file-id="' + id + '"',
      "data-file-id='" + id + "'",
      '/files/' + id
    ];
    for (var i = 0; i < keys.length; i++) {
      if (hay.indexOf(keys[i]) !== -1) return true;
    }
    return false;
  }

  function elementMentionsFileId(el) {
    if (!targetFileId || !el) return false;
    try {
      if (markupHasFileIdToken(hrefOf(el))) return true;
      var data = (el.getAttribute('data-file-id') || '') + '';
      if (data === (targetFileId + '')) return true;
      if (markupHasFileIdToken(el.getAttribute('href') || '')) return true;
    } catch (e) {}
    return false;
  }

  function withinFileRow(el) {
    var node = el;
    var guard = 0;
    while (node && guard < 8) {
      guard += 1;
      var tag = (node.tagName || '').toUpperCase();
      if (tag === 'HEADER' || tag === 'BODY' || tag === 'HTML' || tag === 'NAV') return false;
      if (elementMentionsFileId(node) || markupHasFileIdToken(node.outerHTML || '')) return true;
      node = nextNodeUp(node);
    }
    return false;
  }

  function usableDownloadControl(el) {
    return !!el && rendered(el) && inDownloadSurface(el) && !isAccountOrMarketing(el) && !inSiteChrome(el);
  }

  function findOpenDialogControl() {
    var nodes = candidates().filter(usableDownloadControl);
    var manual = nodes.find(function (el) {
      return isManualDownload(label(el)) && !isFastOrPremium(label(el)) && !isBlockedLabel(label(el));
    });
    if (manual) return { el: manual, action: 'clickManual' };

    if (targetFileId) {
      var nearFile = nodes.find(function (el) {
        var t = label(el);
        if (isFastOrPremium(t) || hasSlowDownloadText(t) || isBlockedLabel(t)) return false;
        if (!(isShortDownload(t) || isManualDownload(t))) return false;
        return withinFileRow(el);
      });
      if (nearFile) return { el: nearFile, action: 'clickFileDownload' };
    }

    var shortDl = nodes.find(function (el) {
      var t = label(el);
      return isShortDownload(t) && !isFastOrPremium(t) && !isBlockedLabel(t);
    });
    if (shortDl) return { el: shortDl, action: 'clickDownload' };

    var modManager = nodes.find(function (el) {
      return isModManager(label(el)) && !isBlockedLabel(label(el));
    });
    if (modManager) return { el: modManager, action: 'clickModManager' };

    return null;
  }

  function hrefHasModId(href) {
    if (!targetModId) return false;
    return new RegExp('/mods/' + targetModId + '(?:[^0-9]|$)').test((href || '') + '');
  }

  function modLinksIn(root) {
    var nodes;
    try {
      nodes = root.querySelectorAll('a[href]');
    } catch (e) {
      return 0;
    }
    var n = 0;
    for (var i = 0; i < nodes.length; i++) {
      if (hrefHasModId(hrefOf(nodes[i]))) n += 1;
    }
    return n;
  }

  function downloadControlIn(root, modLink) {
    var nodes;
    try {
      nodes = root.querySelectorAll('a, button, [role=button], input[type=button], input[type=submit]');
    } catch (e) {
      return null;
    }
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i];
      if (el === modLink || !rendered(el) || isAccountOrMarketing(el) || inSiteChrome(el)) continue;
      var href = hrefOf(el);
      if (isBlockedHref(href)) continue;
      var nxm = nxmHref(el);
      if (nxm) return { el: el, action: 'bridgeNxmHistory', href: nxm };
      if (isCdnHref(href)) return { el: el, action: 'bridgeCdnHistory', href: href };
      var t = label(el);
      if (isFastOrPremium(t) || isBlockedLabel(t)) continue;
      if (hasSlowDownloadText(t) || isManualDownload(t) || isShortDownload(t) || isModManager(t)) {
        return { el: el, action: 'clickHistoryRow' };
      }
    }
    return null;
  }

  function findHistoryRowDownload() {
    if (!targetModId) return null;
    var links = queryDeep('a[href]');
    var fallback = null;
    for (var i = 0; i < links.length; i++) {
      var link = links[i];
      if (inSiteChrome(link) || isAccountOrMarketing(link) || !hrefHasModId(hrefOf(link))) continue;
      var node = link;
      for (var depth = 0; depth < 6 && node; depth++) {
        var tag = (node.tagName || '').toUpperCase();
        if (tag === 'HEADER' || tag === 'NAV' || tag === 'BODY' || tag === 'HTML') break;
        if (modLinksIn(node) > 4) break;
        var choice = downloadControlIn(node, link);
        if (choice) {
          if (elementMentionsFileId(node) || markupHasFileIdToken(node.innerHTML || '')) return choice;
          if (!fallback) fallback = choice;
          break;
        }
        node = node.parentElement;
      }
    }
    return fallback;
  }

  window.__emperorModManagerAutoclickTimer = setInterval(function () {
    if (done || Date.now() > deadline) {
      clearInterval(window.__emperorModManagerAutoclickTimer);
      window.__emperorModManagerAutoclickTimer = null;
      state.phase = 'done';
      touch('expired');
      return;
    }

    if (succeeded()) {
      done = true;
      clearInterval(window.__emperorModManagerAutoclickTimer);
      window.__emperorModManagerAutoclickTimer = null;
      state.phase = 'done';
      touch('succeeded');
      return;
    }

    if (Date.now() - lastClickAt < retryCooldownMs) return;

    state.polls = (state.polls || 0) + 1;
    state.openShadowHosts = 0;
    var body = bodyTextDeep();
    state.bodyHasSlow = body.indexOf('slow download') !== -1;
    state.bodyHasManual = body.indexOf('manual download') !== -1;
    state.iframeCount = countIframesDeep();
    state.candidates = candidates().length;
    state.downloadLabels = collectDownloadLabels();

    if (isHistoryUrl(location.href)) {
      var historyClick = findHistoryRowDownload();
      if (!historyClick) return;
      lastClickAt = Date.now();
      state.phase = 'historyRow';
      if (historyClick.action === 'bridgeNxmHistory') {
        touch('bridgeNxmHistory');
        bridgeNxm(historyClick.href);
        return;
      }
      if (historyClick.action === 'bridgeCdnHistory') {
        touch('bridgeCdnHistory');
        bridgeCdn(historyClick.href);
        return;
      }
      touch('clickHistoryRow');
      realisticClick(historyClick.el);
      return;
    }

    var nxmLinks = queryDeep('a[href^="nxm://"]');
    var nxmLink = null;
    for (var n = 0; n < nxmLinks.length; n++) {
      if (rendered(nxmLinks[n])) { nxmLink = nxmLinks[n]; break; }
    }
    if (!nxmLink) nxmLink = document.querySelector('a[href^="nxm://"]');
    if (nxmLink && rendered(nxmLink) && !inSiteChrome(nxmLink) && !isBlockedHref(hrefOf(nxmLink))) {
      lastClickAt = Date.now();
      state.phase = 'clickSlow';
      touch('bridgeNxm');
      bridgeNxm(nxmHref(nxmLink) || nxmLink.href);
      return;
    }

    var slowCandidate = findSlowDownload();
    state.slowFound = !!slowCandidate;
    state.slowLabel = slowCandidate ? label(slowCandidate) : '';
    state.slowTag = slowCandidate ? ((slowCandidate.tagName || '') + '').toLowerCase() : '';

    if (slowCandidate) {
      state.phase = 'clickSlow';
      lastClickAt = Date.now();
      touch('clickSlow');
      realisticClick(slowCandidate);
      return;
    }

    state.phase = 'openDialog';
    var opener = findOpenDialogControl();
    if (!opener || isAccountOrMarketing(opener.el) || isBlockedHref(hrefOf(opener.el))) return;

    lastClickAt = Date.now();
    var href = nxmHref(opener.el);
    if (href) {
      touch('bridgeNxmModManager');
      bridgeNxm(href);
      return;
    }
    touch(opener.action);
    realisticClick(opener.el);
  }, 400);
})();
"#;

/// Route nxm:// clicks and window.open through an HTTPS sentinel WebKit will actually navigate.
const NXM_HOOK_JS: &str = r#"
(function () {
  function bridge(href) {
    href = (href || '') + '';
    if (href.indexOf('nxm://') !== 0) return false;
    try {
      window.location.href = 'https://emperor-mod-manager.invalid/capture-nxm?url=' + encodeURIComponent(href);
    } catch (err) {}
    return true;
  }
  window.__emperorModManagerBridgeNxm = bridge;
  if (window.__emperorModManagerNxmHook) return;
  window.__emperorModManagerNxmHook = true;
  document.addEventListener('click', function (e) {
    var el = e.target;
    while (el && el.tagName !== 'A') el = el.parentElement;
    if (!el) return;
    var href = (el.href || el.getAttribute('href') || '') + '';
    if (href.indexOf('nxm://') !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    bridge(href);
  }, true);
  var origOpen = window.open;
  window.open = function (url) {
    var href = (url || '') + '';
    if (bridge(href)) return null;
    if (typeof origOpen === 'function') return origOpen.apply(this, arguments);
    return null;
  };
})();
"#;

/// Bridge Nexus CDN download links through an HTTPS sentinel (Slow Download path).
const CDN_HOOK_JS: &str = r#"
(function () {
  function isCdn(href) {
    href = (href || '') + '';
    if (!href || href.indexOf('http') !== 0) return false;
    try {
      var u = new URL(href);
      var h = (u.hostname || '').toLowerCase();
      if (h.indexOf('nexus-cdn') !== -1) return true;
      if (h === 'cf-files.nexusmods.com') return true;
      if (h.indexOf('nexusmods.com') !== -1 && u.pathname.indexOf('/cdn/') !== -1) return true;
    } catch (e) {}
    return false;
  }
  function bridge(href) {
    if (!isCdn(href)) return false;
    try {
      window.location.href = 'https://emperor-mod-manager.invalid/capture-download?url=' + encodeURIComponent(href);
    } catch (err) {}
    return true;
  }
  window.__emperorModManagerBridgeCdn = bridge;
  if (window.__emperorModManagerCdnHook) return;
  window.__emperorModManagerCdnHook = true;
  document.addEventListener('click', function (e) {
    var el = e.target;
    while (el && el.tagName !== 'A') el = el.parentElement;
    if (!el) return;
    var href = (el.href || el.getAttribute('href') || '') + '';
    if (!isCdn(href)) return;
    e.preventDefault();
    e.stopPropagation();
    bridge(href);
  }, true);
  var origOpen = window.open;
  window.open = function (url) {
    var href = (url || '') + '';
    if (bridge(href)) return null;
    if (typeof origOpen === 'function') return origOpen.apply(this, arguments);
    return null;
  };
  function scan() {
    var links = document.querySelectorAll('a[href]');
    for (var i = 0; i < links.length; i++) {
      var href = (links[i].href || links[i].getAttribute('href') || '') + '';
      if (isCdn(href)) {
        bridge(href);
        return;
      }
    }
  }
  scan();
  var started = Date.now();
  var timer = setInterval(function () {
    if (Date.now() - started > 30000) {
      clearInterval(timer);
      return;
    }
    scan();
  }, 500);
})();
"#;

/// What to do with the archive Download Assist captures.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssistPurpose {
    /// Stage into the managed game's mod list (normal free/premium assist).
    #[default]
    StageMod,
    /// Extract legacy Mod Engine (`dinput8.dll`) into the game folder.
    FromsoftLegacyLoader {
        plugin_id: String,
        install_path: String,
    },
    /// Sign in on the Nexus website so later downloads reuse the session cookie.
    WebsiteLogin,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssistContext {
    pub game_id: String,
    pub label: String,
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub generation: u64,
    pub batch_id: Option<String>,
    /// When set, staging replaces this existing staged mod (e.g. free-account update).
    pub replace_staged_id: Option<String>,
    /// Version recorded on the staged row after a free-account install.
    pub version: Option<String>,
    /// File chooser title or archive name, when it differs from the mod title.
    pub file_name: Option<String>,
    pub purpose: AssistPurpose,
}

pub fn set_assist_context(state: &AppState, ctx: AssistContext) {
    if let Ok(mut lock) = state.assist.lock() {
        *lock = Some(ctx);
    }
}

pub fn take_assist_context(state: &AppState) -> Option<AssistContext> {
    state.assist.lock().ok().and_then(|mut g| g.take())
}

pub fn peek_assist_context(state: &AppState) -> Option<AssistContext> {
    state.assist.lock().ok().and_then(|g| g.clone())
}

/// Emit `nxm-url`, debouncing identical URLs within 1s (nav + new_window + scheme + deep link).
pub fn emit_nxm_url(app: &tauri::AppHandle, url: &str) {
    let url = url.trim();
    if !url.starts_with("nxm://") {
        return;
    }
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut last) = state.last_nxm_emit.lock() {
            if let Some((ref prev, at)) = *last {
                if prev == url && at.elapsed() < Duration::from_secs(1) {
                    log::debug!("debounce duplicate nxm emit: {url}");
                    return;
                }
            }
            *last = Some((url.to_string(), Instant::now()));
        }
    }
    log::info!("nxm-url intercepted: {url}");
    let _ = app.emit("nxm-url", url.to_string());
}

pub fn close_assist_webview(app: &tauri::AppHandle) -> Result<(), String> {
    clear_pending_assist_url(app);
    if let Some(w) = app.get_webview(ASSIST_LABEL) {
        w.close().map_err(|e| e.to_string())?;
    }
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    if let Ok(window) = main_window(app) {
        let _ = crate::linux_embed::hide_and_clear(&window);
    }
    Ok(())
}

fn main_window(app: &tauri::AppHandle) -> Result<tauri::Window, String> {
    app.get_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| format!("Main window '{MAIN_WINDOW_LABEL}' not found"))
}

fn stored_bounds(state: &AppState) -> AssistBounds {
    state
        .assist_bounds
        .lock()
        .ok()
        .map(|g| *g)
        .unwrap_or_default()
}

fn clear_pending_assist_url(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut pending) = state.assist_pending_url.lock() {
            *pending = None;
        }
    }
}

fn store_pending_assist_url(state: &AppState, url: &url::Url) {
    if let Ok(mut pending) = state.assist_pending_url.lock() {
        *pending = Some(url.to_string());
    }
}

fn flush_pending_assist_navigation(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let raw = state
        .assist_pending_url
        .lock()
        .ok()
        .and_then(|mut pending| pending.take());
    let Some(raw) = raw else {
        return Ok(());
    };
    let url = url::Url::parse(&raw).map_err(|e| e.to_string())?;
    if let Some(webview) = app.get_webview(ASSIST_LABEL) {
        webview.navigate(url).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn apply_assist_bounds(
    app: &tauri::AppHandle,
    webview: &Webview,
    bounds: AssistBounds,
) -> Result<(), String> {
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        let _ = webview;
        crate::linux_embed::apply_bounds(&main_window(app)?, bounds)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    {
        webview
            .set_bounds(Rect {
                position: LogicalPosition::new(bounds.x, bounds.y).into(),
                size: LogicalSize::new(bounds.width.max(120.0), bounds.height.max(80.0)).into(),
            })
            .map_err(|e| e.to_string())
    }
}

/// Take context, mark intentional close for this generation, and close the embedded webview.
pub fn close_assist_intentionally(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let gen = peek_assist_context(state)
        .map(|c| c.generation)
        .unwrap_or_else(|| state.assist_window_gen.load(Ordering::SeqCst));
    state.assist_closed_gen.store(gen, Ordering::SeqCst);
    let _ = take_assist_context(state);
    close_assist_webview(app)
}

async fn wait_assist_webview_gone(app: &tauri::AppHandle) {
    for _ in 0..50 {
        if app.get_webview(ASSIST_LABEL).is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = close_assist_webview(app);
    for _ in 0..20 {
        if app.get_webview(ASSIST_LABEL).is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn assist_files_url(domain: &str, mod_id: u64, file_id: u64) -> Result<url::Url, String> {
    let url =
        format!("https://www.nexusmods.com/{domain}/mods/{mod_id}?tab=files&file_id={file_id}");
    url.parse().map_err(|e: url::ParseError| e.to_string())
}

fn is_nexus_login_url(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    if !u.contains("nexusmods.com") {
        return false;
    }
    u.contains("/auth/") || u.contains("sign-in") || u.contains("signin") || u.contains("/login")
}

fn has_nexus_session_cookie(cookie_names: &[String]) -> bool {
    cookie_names
        .iter()
        .any(|name| name.eq_ignore_ascii_case("nexusmods_session"))
}

/// Logged-in Nexus page with the website session cookie Download Assist reuses.
fn nexus_website_login_succeeded(url: &str, cookie_names: &[String]) -> bool {
    url.to_ascii_lowercase().contains("nexusmods.com")
        && !is_nexus_login_url(url)
        && has_nexus_session_cookie(cookie_names)
}

fn nexus_sign_in_url() -> Result<url::Url, String> {
    "https://users.nexusmods.com/auth/sign_in?redirect_url=https%3A%2F%2Fwww.nexusmods.com%2F"
        .parse()
        .map_err(|e: url::ParseError| e.to_string())
}

fn download_assist_in_use(state: &AppState) -> bool {
    matches!(
        peek_assist_context(state).as_ref().map(|c| &c.purpose),
        Some(AssistPurpose::StageMod) | Some(AssistPurpose::FromsoftLegacyLoader { .. })
    )
}

fn intercept_login_navigation(nav_url: &url::Url) -> bool {
    let s = nav_url.as_str();
    if s.starts_with("nxm://") || is_nexus_cdn_download(nav_url) {
        return false;
    }
    nav_url.host_str() != Some(NXM_SENTINEL_HOST)
}

fn schedule_nexus_login_close(app: tauri::AppHandle, generation: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let still = peek_assist_context(&state).is_some_and(|c| {
            c.generation == generation && matches!(c.purpose, AssistPurpose::WebsiteLogin)
        });
        if !still {
            return;
        }
        let _ = app.emit("nexus-login-complete", ());
        let _ = close_assist_intentionally(&app, &state);
    });
}

fn poll_nexus_login_cookie(app: tauri::AppHandle, generation: u64, done: Arc<AtomicBool>) {
    tauri::async_runtime::spawn(async move {
        let Ok(cookie_url) = url::Url::parse("https://www.nexusmods.com/") else {
            return;
        };
        for _ in 0..12 {
            if done.load(Ordering::SeqCst) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            if done.load(Ordering::SeqCst) {
                return;
            }
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            let still = peek_assist_context(&state).is_some_and(|c| {
                c.generation == generation && matches!(c.purpose, AssistPurpose::WebsiteLogin)
            });
            if !still {
                return;
            }
            let Some(webview) = app.get_webview(ASSIST_LABEL) else {
                return;
            };
            let Ok(current) = webview.url() else {
                continue;
            };
            let Ok(cookies) = webview.cookies_for_url(cookie_url.clone()) else {
                continue;
            };
            let names = cookies
                .iter()
                .map(|cookie| cookie.name().to_string())
                .collect::<Vec<_>>();
            if nexus_website_login_succeeded(current.as_str(), &names)
                && !done.swap(true, Ordering::SeqCst)
            {
                schedule_nexus_login_close(app, generation);
                return;
            }
        }
    });
}

fn emit_assist_opened(
    app: &tauri::AppHandle,
    autoclick: bool,
    domain: &str,
    mod_id: u64,
    file_id: u64,
) {
    let _ = app.emit(
        "assist-opened",
        serde_json::json!({
            "autoclick": autoclick,
            "mod_id": mod_id,
            "file_id": file_id,
            "domain": domain,
        }),
    );
}

fn maybe_emit_login_needed(app: &tauri::AppHandle, url: &str) {
    if is_nexus_login_url(url) {
        let _ = app.emit("assist-needs-login", ());
    }
}

fn autoclick_enabled(state: &AppState) -> bool {
    state
        .config
        .lock()
        .ok()
        .map(|c| c.autoclick_free_download)
        .unwrap_or(false)
}

fn normalize_nxm_request_uri(uri: &str) -> Option<String> {
    let uri = uri.trim();
    if uri.starts_with("nxm://") {
        return Some(uri.to_string());
    }
    // Some webviews pass scheme-relative forms.
    if let Some(rest) = uri.strip_prefix("nxm:") {
        let rest = rest.trim_start_matches('/');
        return Some(format!("nxm://{rest}"));
    }
    None
}

pub fn is_nexus_cdn_download(url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    host.ends_with(".nexus-cdn.com")
        || host == "cf-files.nexusmods.com"
        || (host.ends_with(".nexusmods.com") && url.path().contains("/cdn/"))
}

fn extract_sentinel_download(nav_url: &url::Url) -> Option<String> {
    if nav_url.scheme() != "https" {
        return None;
    }
    if nav_url.host_str() != Some(NXM_SENTINEL_HOST) {
        return None;
    }
    if nav_url.path() != CDN_SENTINEL_PATH {
        return None;
    }
    let encoded = nav_url
        .query_pairs()
        .find(|(k, _)| k == "url")
        .map(|(_, v)| v.into_owned())?;
    let decoded = encoded.trim();
    let parsed = url::Url::parse(decoded).ok()?;
    if is_nexus_cdn_download(&parsed) {
        Some(decoded.to_string())
    } else {
        None
    }
}

fn spawn_cdn_download(app: &tauri::AppHandle, cdn_url: String) {
    log::info!("cdn download intercepted: {cdn_url}");
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(state) = app.try_state::<AppState>() {
            match crate::commands::start_assist_cdn_download(&app, &state, &cdn_url).await {
                Ok(_) => log::debug!("cdn download started successfully"),
                Err(e) if e == "DUPLICATE_CDN" => log::debug!("debounce duplicate cdn: {cdn_url}"),
                Err(e) => log::error!("cdn download failed to start: {e}"),
            }
        }
    });
}

fn extract_sentinel_nxm(nav_url: &url::Url) -> Option<String> {
    if nav_url.scheme() != "https" {
        return None;
    }
    if nav_url.host_str() != Some(NXM_SENTINEL_HOST) {
        return None;
    }
    if nav_url.path() != NXM_SENTINEL_PATH {
        return None;
    }
    let encoded = nav_url
        .query_pairs()
        .find(|(k, _)| k == "url")
        .map(|(_, v)| v.into_owned())?;
    let decoded = encoded.trim();
    if decoded.starts_with("nxm://") {
        Some(decoded.to_string())
    } else {
        None
    }
}

fn intercept_assist_navigation(app: &tauri::AppHandle, nav_url: &url::Url) -> bool {
    let s = nav_url.as_str();
    if s.starts_with("nxm://") {
        log::info!("assist navigation captured nxm:// directly");
        emit_nxm_url(app, s);
        return false;
    }
    if is_nexus_cdn_download(nav_url) {
        spawn_cdn_download(app, s.to_string());
        return false;
    }
    if nav_url.host_str() == Some(NXM_SENTINEL_HOST) {
        if let Some(nxm) = extract_sentinel_nxm(nav_url) {
            log::info!("assist sentinel captured nxm://: {nxm}");
            emit_nxm_url(app, &nxm);
            return false;
        }
        if let Some(cdn) = extract_sentinel_download(nav_url) {
            log::info!("assist sentinel captured cdn: {cdn}");
            spawn_cdn_download(app, cdn);
            return false;
        }
        log::warn!("assist sentinel rejected payload: {s}");
        return false;
    }
    maybe_emit_login_needed(app, s);
    true
}

fn eval_assist_script(webview: &Webview, href: &str, name: &str, script: &str) {
    if let Err(e) = webview.eval(script) {
        log::warn!("assist {name} eval failed on {href}: {e}");
    }
}

fn schedule_autoclick_telemetry(webview: Webview, href: String) {
    tauri::async_runtime::spawn(async move {
        for attempt in 1..=30u32 {
            tokio::time::sleep(Duration::from_millis(2000)).await;
            let href_log = href.clone();
            let _ = webview.eval_with_callback(
                "JSON.stringify(window.__emperorModManagerAutoclickState || { missing: true })",
                move |result| {
                    log::info!("autoclick telemetry attempt {attempt} on {href_log}: {result}");
                },
            );
        }
    });
}

fn assist_context_js(mod_id: u64, file_id: u64) -> String {
    format!("window.__emperorModManagerAssist = {{ fileId: {file_id}, modId: {mod_id} }};")
}

fn inject_assist_scripts(
    webview: &Webview,
    href: &str,
    autoclick: bool,
    mod_id: u64,
    file_id: u64,
) {
    log::debug!("assist injecting nxm + cdn bridges on {href}");
    eval_assist_script(
        webview,
        href,
        "assist context",
        &assist_context_js(mod_id, file_id),
    );
    eval_assist_script(webview, href, "nxm hook", NXM_HOOK_JS);
    eval_assist_script(webview, href, "cdn hook", CDN_HOOK_JS);
    if autoclick {
        eval_assist_script(webview, href, "autoclick", AUTOCLICK_JS);
        schedule_autoclick_telemetry(webview.clone(), href.to_string());
    }
}

fn apply_assist_desired_visible(app: &tauri::AppHandle, state: &AppState) -> Result<(), String> {
    let visible = state.assist_desired_visible.load(Ordering::SeqCst);
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        // Opacity and pass-through hide the overlay child. Unmapping WebKit
        // leaves the next show blank, and a 1×1 first allocation sticks.
        if let Ok(window) = main_window(app) {
            crate::linux_embed::set_visible(&window, visible)?;
        }
        if visible {
            flush_pending_assist_navigation(app, state)?;
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    if let Some(webview) = app.get_webview(ASSIST_LABEL) {
        if visible {
            webview.show().map_err(|e| e.to_string())?;
        } else {
            webview.hide().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn set_assist_bounds(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let bounds = AssistBounds {
        x,
        y,
        width,
        height,
    };
    if let Ok(mut lock) = state.assist_bounds.lock() {
        *lock = bounds;
    }
    if let Some(webview) = app.get_webview(ASSIST_LABEL) {
        apply_assist_bounds(&app, &webview, bounds)?;
        // Bounds must not force-show; respect Downloads-tab desired visibility.
        apply_assist_desired_visible(&app, &state)?;
    }
    Ok(())
}

#[tauri::command]
pub fn set_assist_visible(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    visible: bool,
) -> Result<(), String> {
    state
        .assist_desired_visible
        .store(visible, Ordering::SeqCst);
    apply_assist_desired_visible(&app, &state)
}

#[tauri::command]
pub async fn open_download_assist(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    game_id: String,
    domain: String,
    mod_id: u64,
    file_id: u64,
    name: String,
    batch_id: Option<String>,
    replace_staged_id: Option<String>,
    version: Option<String>,
    file_name: Option<String>,
    purpose: Option<AssistPurpose>,
) -> Result<(), String> {
    open_download_assist_inner(
        &app,
        &state,
        game_id,
        domain,
        mod_id,
        file_id,
        name,
        batch_id,
        replace_staged_id,
        version,
        file_name,
        purpose.unwrap_or_default(),
    )
    .await
}

/// Open Download Assist with an explicit purpose (used by FromSoft loader install).
pub async fn open_download_assist_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    game_id: String,
    domain: String,
    mod_id: u64,
    file_id: u64,
    name: String,
    batch_id: Option<String>,
    replace_staged_id: Option<String>,
    version: Option<String>,
    file_name: Option<String>,
    purpose: AssistPurpose,
) -> Result<(), String> {
    // Close → wait → open next so SPA state / autoclick flags cannot leak across mods.
    if app.get_webview(ASSIST_LABEL).is_some() {
        let gen = peek_assist_context(state)
            .map(|c| c.generation)
            .unwrap_or_else(|| state.assist_window_gen.load(Ordering::SeqCst));
        state.assist_closed_gen.store(gen, Ordering::SeqCst);
        let _ = take_assist_context(state);
        let _ = close_assist_webview(app);
        wait_assist_webview_gone(app).await;
    }

    let generation = state.assist_window_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let autoclick = autoclick_enabled(state);
    let bounds = stored_bounds(state);

    set_assist_context(
        state,
        AssistContext {
            game_id,
            label: name.clone(),
            domain: domain.clone(),
            mod_id,
            file_id,
            generation,
            batch_id,
            replace_staged_id,
            version,
            file_name,
            purpose,
        },
    );

    let parsed = assist_files_url(&domain, mod_id, file_id)?;

    let assist_dir = state.paths.assist_webview_dir();
    fs::create_dir_all(&assist_dir).map_err(|e| e.to_string())?;

    let handle_nav = app.clone();
    let download_dir = state.paths.downloads_dir();
    let _ = fs::create_dir_all(&download_dir);

    let window = main_window(app)?;

    // On Linux the child is reparented after creation. Loading Nexus before that
    // attaches WebKit's surface to the window box and the page freezes.
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    let initial_url = WebviewUrl::External(
        "about:blank"
            .parse()
            .map_err(|e: url::ParseError| e.to_string())?,
    );
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    let initial_url = WebviewUrl::External(parsed.clone());

    let mut builder = WebviewBuilder::new(ASSIST_LABEL, initial_url)
        .data_directory(assist_dir)
        .initialization_script(assist_context_js(mod_id, file_id))
        .initialization_script(NXM_HOOK_JS)
        .initialization_script(CDN_HOOK_JS)
        .on_navigation(move |nav_url| intercept_assist_navigation(&handle_nav, nav_url));

    if autoclick {
        builder = builder.initialization_script(AUTOCLICK_JS);
    }

    let handle_new = app.clone();
    builder = builder.on_new_window(move |url, _features| {
        if !intercept_assist_navigation(&handle_new, &url) {
            return NewWindowResponse::Deny;
        }
        if url
            .host_str()
            .map(|h| h.eq_ignore_ascii_case("nexusmods.com") || h.ends_with(".nexusmods.com"))
            .unwrap_or(false)
        {
            return NewWindowResponse::Allow;
        }
        NewWindowResponse::Deny
    });

    let dl_dir = download_dir.clone();
    let handle_dl = app.clone();
    builder = builder.on_download(move |_webview, event| match event {
        DownloadEvent::Requested { url, destination } => {
            if is_nexus_cdn_download(&url) {
                log::info!("assist on_download captured cdn: {url}");
                spawn_cdn_download(&handle_dl, url.to_string());
                return false;
            }
            let name = destination
                .file_name()
                .map(|s| s.to_owned())
                .unwrap_or_else(|| std::ffi::OsString::from("nexus-download.bin"));
            *destination = dl_dir.join(name);
            true
        }
        DownloadEvent::Finished { path, success, .. } => {
            if success {
                if let Some(path) = path {
                    let _ = handle_dl.emit(
                        "assist-download-finished",
                        path.to_string_lossy().to_string(),
                    );
                }
            }
            true
        }
        _ => true,
    });

    let handle_load = app.clone();
    let autoclick_for_load = autoclick;
    let load_mod_id = mod_id;
    let load_file_id = file_id;
    builder = builder.on_page_load(move |webview, payload| {
        if payload.event() != PageLoadEvent::Finished {
            return;
        }
        let href = payload.url().as_str().to_string();
        if !href.contains("nexusmods.com") {
            return;
        }
        maybe_emit_login_needed(&handle_load, &href);
        if is_nexus_login_url(&href) {
            return;
        }
        let enable = handle_load
            .try_state::<AppState>()
            .map(|s| autoclick_enabled(&s))
            .unwrap_or(autoclick_for_load);
        inject_assist_scripts(&webview, &href, enable, load_mod_id, load_file_id);
    });

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        crate::linux_embed::prepare(&window)?;
        // Inherited by the assist web process started below. The main UI
        // webview is already running and keeps its own compositor.
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let webview = window
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width, bounds.height),
        )
        .map_err(|e| e.to_string())?;

    // Loader installs run mid-deploy: force the panel visible. Normal assist still
    // respects the Downloads-tab desired-visible flag.
    if matches!(
        peek_assist_context(state).as_ref().map(|c| &c.purpose),
        Some(AssistPurpose::FromsoftLegacyLoader { .. })
    ) {
        state.assist_desired_visible.store(true, Ordering::SeqCst);
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    crate::linux_embed::attach_assist(
        &window,
        bounds,
        state.assist_desired_visible.load(Ordering::SeqCst),
    )?;

    apply_assist_bounds(app, &webview, bounds)?;
    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    store_pending_assist_url(state, &parsed);
    // Flushes the pending URL when Downloads already wants the panel visible.
    apply_assist_desired_visible(app, state)?;

    emit_assist_opened(app, autoclick, &domain, mod_id, file_id);
    Ok(())
}

#[tauri::command]
pub fn close_download_assist(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    close_assist_intentionally(&app, &state)
}

/// Open the Nexus sign-in page in the Download Assist profile.
///
/// The WebKit data directory is the same one later downloads read cookies from.
/// The webview closes after `nexusmods_session` is stored.
#[tauri::command]
pub async fn open_nexus_website_login(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if download_assist_in_use(&state) {
        return Err(
            "Download Assist is in use. Finish or cancel the queue before signing in.".into(),
        );
    }

    if app.get_webview(ASSIST_LABEL).is_some() {
        let gen = peek_assist_context(&state)
            .map(|c| c.generation)
            .unwrap_or_else(|| state.assist_window_gen.load(Ordering::SeqCst));
        state.assist_closed_gen.store(gen, Ordering::SeqCst);
        let _ = take_assist_context(&state);
        let _ = close_assist_webview(&app);
        wait_assist_webview_gone(&app).await;
    }

    let generation = state.assist_window_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let bounds = stored_bounds(&state);
    set_assist_context(
        &state,
        AssistContext {
            game_id: String::new(),
            label: "Nexus sign-in".into(),
            domain: String::new(),
            mod_id: 0,
            file_id: 0,
            generation,
            batch_id: None,
            replace_staged_id: None,
            version: None,
            file_name: None,
            purpose: AssistPurpose::WebsiteLogin,
        },
    );

    let parsed = nexus_sign_in_url()?;
    let assist_dir = state.paths.assist_webview_dir();
    fs::create_dir_all(&assist_dir).map_err(|e| e.to_string())?;
    let window = main_window(&app)?;

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    let initial_url = WebviewUrl::External(
        "about:blank"
            .parse()
            .map_err(|e: url::ParseError| e.to_string())?,
    );
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    let initial_url = WebviewUrl::External(parsed.clone());

    let mut builder = WebviewBuilder::new(ASSIST_LABEL, initial_url)
        .data_directory(assist_dir)
        .on_navigation(|nav_url| intercept_login_navigation(nav_url));

    let handle_new = app.clone();
    builder = builder.on_new_window(move |url, _features| {
        if let Some(webview) = handle_new.get_webview(ASSIST_LABEL) {
            let _ = webview.navigate(url);
        }
        NewWindowResponse::Deny
    });

    builder = builder.on_download(|_webview, _event| false);

    let handle_load = app.clone();
    let done = Arc::new(AtomicBool::new(false));
    builder = builder.on_page_load(move |webview, payload| {
        if payload.event() != PageLoadEvent::Finished {
            return;
        }
        let href = payload.url().as_str().to_string();
        if is_nexus_login_url(&href) || !href.to_ascii_lowercase().contains("nexusmods.com") {
            return;
        }
        let Ok(cookie_url) = url::Url::parse("https://www.nexusmods.com/") else {
            return;
        };
        let names = webview
            .cookies_for_url(cookie_url)
            .map(|cookies| {
                cookies
                    .iter()
                    .map(|cookie| cookie.name().to_string())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if nexus_website_login_succeeded(&href, &names) {
            if !done.swap(true, Ordering::SeqCst) {
                schedule_nexus_login_close(handle_load.clone(), generation);
            }
            return;
        }
        poll_nexus_login_cookie(handle_load.clone(), generation, done.clone());
    });

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        crate::linux_embed::prepare(&window)?;
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    let webview = match window.add_child(
        builder,
        LogicalPosition::new(bounds.x, bounds.y),
        LogicalSize::new(bounds.width, bounds.height),
    ) {
        Ok(webview) => webview,
        Err(e) => {
            let _ = take_assist_context(&state);
            return Err(e.to_string());
        }
    };

    state.assist_desired_visible.store(true, Ordering::SeqCst);

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    if let Err(e) = crate::linux_embed::attach_assist(&window, bounds, true) {
        let _ = take_assist_context(&state);
        let _ = close_assist_webview(&app);
        return Err(e);
    }

    if let Err(e) = apply_assist_bounds(&app, &webview, bounds) {
        let _ = take_assist_context(&state);
        let _ = close_assist_webview(&app);
        return Err(e);
    }

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    store_pending_assist_url(&state, &parsed);
    apply_assist_desired_visible(&app, &state)?;
    Ok(())
}

/// Close Download Assist and delete its WebKit profile (Nexus website cookies).
#[tauri::command]
pub fn clear_assist_session(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    close_assist_intentionally(&app, &state)?;

    let dir = state.paths.assist_webview_dir();
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("clear assist session: {e}"))?;
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(())
}

/// Used by the app-level `nxm` URI scheme protocol.
pub fn handle_nxm_uri_scheme(app: &tauri::AppHandle, request_uri: &str) {
    if let Some(url) = normalize_nxm_request_uri(request_uri) {
        emit_nxm_url(app, &url);
    } else {
        log::warn!("unrecognized nxm uri-scheme request: {request_uri}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sentinel_nxm_url() {
        let raw = "https://emperor-mod-manager.invalid/capture-nxm?url=nxm%3A%2F%2Fstardewvalley%2Fmods%2F2400%2Ffiles%2F12345%3Fkey%3Dabc%26expires%3D1";
        let parsed = url::Url::parse(raw).unwrap();
        let nxm = extract_sentinel_nxm(&parsed).unwrap();
        assert_eq!(
            nxm,
            "nxm://stardewvalley/mods/2400/files/12345?key=abc&expires=1"
        );
    }

    #[test]
    fn rejects_non_nxm_sentinel_payload() {
        let parsed = url::Url::parse(
            "https://emperor-mod-manager.invalid/capture-nxm?url=https://example.com",
        )
        .unwrap();
        assert!(extract_sentinel_nxm(&parsed).is_none());
    }

    #[test]
    fn ignores_unrelated_https_urls() {
        let parsed = url::Url::parse("https://www.nexusmods.com/stardewvalley/mods/1").unwrap();
        assert!(extract_sentinel_nxm(&parsed).is_none());
    }

    #[test]
    fn detects_nexus_cdn_hosts() {
        assert!(is_nexus_cdn_download(
            &url::Url::parse("https://cf-files.nexusmods.com/cdn/1704/1137/mod.zip").unwrap()
        ));
        assert!(is_nexus_cdn_download(
            &url::Url::parse("https://files.nexus-cdn.com/cdn/110/607/mod.zip").unwrap()
        ));
        assert!(!is_nexus_cdn_download(
            &url::Url::parse("https://www.nexusmods.com/cyberpunk2077/mods/1").unwrap()
        ));
    }

    #[test]
    fn extracts_sentinel_cdn_url() {
        let raw = "https://emperor-mod-manager.invalid/capture-download?url=https%3A%2F%2Fcf-files.nexusmods.com%2Fcdn%2F1%2F2%2Fmod.zip";
        let parsed = url::Url::parse(raw).unwrap();
        let cdn = extract_sentinel_download(&parsed).unwrap();
        assert_eq!(cdn, "https://cf-files.nexusmods.com/cdn/1/2/mod.zip");
    }

    #[test]
    fn rejects_non_cdn_sentinel_payload() {
        let parsed = url::Url::parse(
            "https://emperor-mod-manager.invalid/capture-download?url=https://example.com/file.zip",
        )
        .unwrap();
        assert!(extract_sentinel_download(&parsed).is_none());
    }

    #[test]
    fn autoclick_script_is_two_phase() {
        assert!(AUTOCLICK_JS.contains("retryCooldownMs = 800"));
        assert!(AUTOCLICK_JS.contains("findSlowDownload"));
        assert!(AUTOCLICK_JS.contains("findOpenDialogControl"));
        assert!(AUTOCLICK_JS.contains("queryDeep"));
        assert!(AUTOCLICK_JS.contains("shadowRoot"));
        assert!(AUTOCLICK_JS.contains("bodyTextDeep"));
        assert!(AUTOCLICK_JS.contains("nxm-button-secondary-filled-weak"));
        assert!(AUTOCLICK_JS.contains("__emperorModManagerAssist"));
        assert!(AUTOCLICK_JS.contains("openDialog"));
        assert!(AUTOCLICK_JS.contains("clickSlow"));
        assert!(AUTOCLICK_JS.contains("bodyHasSlow"));
        assert!(AUTOCLICK_JS.contains("bodyHasManual"));
        assert!(AUTOCLICK_JS.contains("deepNxmWeakCount"));
        assert!(AUTOCLICK_JS.contains("deepHasSlow"));
        assert!(AUTOCLICK_JS.contains("iframeCount"));
        assert!(AUTOCLICK_JS.contains("openShadowHosts"));
        assert!(AUTOCLICK_JS.contains("manual download"));
        assert!(AUTOCLICK_JS.contains("nxmButtonCount"));
        assert!(AUTOCLICK_JS.contains("realisticClick"));
        assert!(AUTOCLICK_JS.contains("your download is starting"));
        assert!(AUTOCLICK_JS.contains("tab=download+history"));
        assert!(AUTOCLICK_JS.contains("download-history"));
        assert!(AUTOCLICK_JS.contains("next.nexusmods.com/vortex"));
        assert!(AUTOCLICK_JS.contains("download history"));
        assert!(AUTOCLICK_JS.contains("download app"));
        assert!(AUTOCLICK_JS.contains("visuallyHidden"));
        assert!(AUTOCLICK_JS.contains("inDownloadSurface"));
        assert!(AUTOCLICK_JS.contains("markupHasFileIdToken"));
        assert!(AUTOCLICK_JS.contains("findHistoryRowDownload"));
        assert!(AUTOCLICK_JS.contains("clickHistoryRow"));
        assert!(AUTOCLICK_JS.contains("isHistoryUrl"));
        assert!(!AUTOCLICK_JS.contains("innerHeight"));
        assert!(!AUTOCLICK_JS.contains("innerWidth"));
        assert!(!AUTOCLICK_JS.contains("clickVortex"));
    }

    #[test]
    fn assist_context_js_embeds_ids() {
        let js = assist_context_js(3518, 63684);
        assert!(js.contains("fileId: 63684"));
        assert!(js.contains("modId: 3518"));
    }

    #[test]
    fn assist_purpose_defaults_to_stage_mod() {
        assert!(matches!(AssistPurpose::default(), AssistPurpose::StageMod));
        let json = serde_json::to_string(&AssistPurpose::default()).unwrap();
        assert_eq!(json, "\"stage_mod\"");
    }

    #[test]
    fn website_login_succeeds_only_after_leaving_sign_in() {
        let session = vec!["nexusmods_session".to_string()];
        assert!(!nexus_website_login_succeeded(
            "https://users.nexusmods.com/auth/sign_in",
            &session
        ));
        assert!(!nexus_website_login_succeeded(
            "https://users.nexusmods.com/auth/two-factor",
            &session
        ));
        assert!(!nexus_website_login_succeeded(
            "https://www.nexusmods.com/",
            &[]
        ));
        assert!(nexus_website_login_succeeded(
            "https://www.nexusmods.com/",
            &session
        ));
        assert!(!nexus_website_login_succeeded(
            "https://accounts.google.com/",
            &session
        ));
    }
}
