import DOMPurify from "dompurify";

const SIZE_MAP: Record<string, string> = {
  "1": "0.75em",
  "2": "0.875em",
  "3": "1em",
  "4": "1.125em",
  "5": "1.375em",
  "6": "1.75em",
};

const NAMED_COLORS = new Set(
  [
    "red",
    "green",
    "blue",
    "orange",
    "yellow",
    "purple",
    "black",
    "white",
    "gray",
    "grey",
    "pink",
    "cyan",
    "magenta",
    "brown",
    "navy",
    "teal",
    "olive",
    "maroon",
    "silver",
    "lime",
    "aqua",
    "fuchsia",
  ].map((c) => c.toLowerCase()),
);

const ALLOWED_TAGS = [
  "p",
  "br",
  "a",
  "ul",
  "ol",
  "li",
  "strong",
  "b",
  "em",
  "i",
  "u",
  "s",
  "strike",
  "del",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "blockquote",
  "code",
  "pre",
  "img",
  "span",
  "div",
  "hr",
  "details",
  "summary",
];

const ALLOWED_ATTR = ["href", "src", "alt", "title", "target", "rel", "style", "class"];

let hooksInstalled = false;

function ensureHooks() {
  if (hooksInstalled) return;
  hooksInstalled = true;
  DOMPurify.addHook("afterSanitizeAttributes", (node) => {
    if (node.tagName === "A") {
      const href = node.getAttribute("href");
      if (!href || !isSafeUrl(href)) {
        node.removeAttribute("href");
      } else {
        node.setAttribute("target", "_blank");
        node.setAttribute("rel", "noopener noreferrer");
      }
    }
    if (node.tagName === "IMG") {
      const src = node.getAttribute("src");
      if (!src || !isSafeUrl(src)) {
        node.removeAttribute("src");
      }
    }
    if (node.hasAttribute("style")) {
      const cleaned = sanitizeInlineStyle(node.getAttribute("style") ?? "");
      if (cleaned) node.setAttribute("style", cleaned);
      else node.removeAttribute("style");
    }
  });
}

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function looksLikeHtml(text: string): boolean {
  return /<[a-z][\s\S]*>/i.test(text);
}

export function looksLikeBbcode(text: string): boolean {
  return /\[[a-z*][^[\]]*\]/i.test(text);
}

export function isSafeUrl(url: string): boolean {
  return /^https?:\/\//i.test(url.trim());
}

export function isSafeColor(color: string): boolean {
  const c = color.trim();
  if (/^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(c)) return true;
  return NAMED_COLORS.has(c.toLowerCase());
}

function sanitizeInlineStyle(style: string): string {
  const parts: string[] = [];
  for (const decl of style.split(";")) {
    const [rawProp, ...rest] = decl.split(":");
    if (!rawProp || rest.length === 0) continue;
    const prop = rawProp.trim().toLowerCase();
    const value = rest.join(":").trim();
    if (!value || /expression|url\s*\(|javascript:/i.test(value)) continue;
    if (prop === "color" && isSafeColor(value)) {
      parts.push(`color: ${value}`);
    } else if (prop === "font-size" && /^[\d.]+\s*(em|rem|px|%)$/i.test(value)) {
      parts.push(`font-size: ${value}`);
    } else if (
      prop === "text-align" &&
      /^(left|right|center|justify)$/i.test(value)
    ) {
      parts.push(`text-align: ${value.toLowerCase()}`);
    }
  }
  return parts.join("; ");
}

function sizeToCss(size: string): string | null {
  const key = size.trim();
  if (SIZE_MAP[key]) return SIZE_MAP[key];
  if (/^\d{1,3}$/.test(key)) {
    const n = Number(key);
    if (n >= 8 && n <= 72) return `${n}px`;
  }
  return null;
}

function replaceRepeated(
  input: string,
  pattern: RegExp,
  replacer: (match: string, ...groups: string[]) => string,
): string {
  let text = input;
  const flags = pattern.flags.includes("g") ? pattern.flags : `${pattern.flags}g`;
  const re = new RegExp(pattern.source, flags);
  for (let i = 0; i < 20; i++) {
    const next = text.replace(re, replacer);
    if (next === text) break;
    text = next;
  }
  return text;
}

/** Convert Nexus-style BBCode into HTML. Leaves existing HTML tags intact. */
export function bbcodeToHtml(input: string): string {
  let text = input.replace(/\r\n/g, "\n").replace(/\r/g, "\n");

  // Code blocks: escape inner content and skip further BBCode inside.
  text = text.replace(/\[code\]([\s\S]*?)\[\/code\]/gi, (_m, body: string) => {
    return `<pre><code>${escapeHtml(body.replace(/^\n+|\n+$/g, ""))}</code></pre>`;
  });

  text = text.replace(/\[youtube\]([a-zA-Z0-9_-]{6,20})\[\/youtube\]/gi, (_m, id: string) => {
    const href = `https://www.youtube.com/watch?v=${id}`;
    return `<a href="${href}">YouTube: ${id}</a>`;
  });

  text = text.replace(/\[img\]\s*(https?:\/\/[^[\]]+?)\s*\[\/img\]/gi, (_m, url: string) => {
    const clean = url.trim();
    if (!isSafeUrl(clean)) return "";
    return `<img src="${escapeHtml(clean)}" alt="">`;
  });

  text = text.replace(
    /\[url=(https?:\/\/[^[\]]+?)\]([\s\S]*?)\[\/url\]/gi,
    (_m, url: string, label: string) => {
      const clean = url.trim();
      if (!isSafeUrl(clean)) return label;
      return `<a href="${escapeHtml(clean)}">${label}</a>`;
    },
  );

  text = text.replace(/\[url\]\s*(https?:\/\/[^[\]]+?)\s*\[\/url\]/gi, (_m, url: string) => {
    const clean = url.trim();
    if (!isSafeUrl(clean)) return escapeHtml(clean);
    return `<a href="${escapeHtml(clean)}">${escapeHtml(clean)}</a>`;
  });

  text = replaceRepeated(text, /\[b\]([\s\S]*?)\[\/b\]/gi, (_m, body) => `<strong>${body}</strong>`);
  text = replaceRepeated(text, /\[i\]([\s\S]*?)\[\/i\]/gi, (_m, body) => `<em>${body}</em>`);
  text = replaceRepeated(text, /\[u\]([\s\S]*?)\[\/u\]/gi, (_m, body) => `<u>${body}</u>`);
  text = replaceRepeated(text, /\[s\]([\s\S]*?)\[\/s\]/gi, (_m, body) => `<s>${body}</s>`);
  text = replaceRepeated(
    text,
    /\[strike\]([\s\S]*?)\[\/strike\]/gi,
    (_m, body) => `<s>${body}</s>`,
  );

  text = replaceRepeated(
    text,
    /\[color=([^\]]+)\]([\s\S]*?)\[\/color\]/gi,
    (_m, colorRaw, body) => {
      const color = colorRaw.trim();
      if (!isSafeColor(color)) return body;
      return `<span style="color: ${color}">${body}</span>`;
    },
  );

  text = replaceRepeated(
    text,
    /\[size=([^\]]+)\]([\s\S]*?)\[\/size\]/gi,
    (_m, sizeRaw, body) => {
      const css = sizeToCss(sizeRaw);
      if (!css) return body;
      return `<span style="font-size: ${css}">${body}</span>`;
    },
  );

  text = replaceRepeated(
    text,
    /\[center\]([\s\S]*?)\[\/center\]/gi,
    (_m, body) => `<div style="text-align: center">${body}</div>`,
  );
  text = replaceRepeated(
    text,
    /\[left\]([\s\S]*?)\[\/left\]/gi,
    (_m, body) => `<div style="text-align: left">${body}</div>`,
  );
  text = replaceRepeated(
    text,
    /\[right\]([\s\S]*?)\[\/right\]/gi,
    (_m, body) => `<div style="text-align: right">${body}</div>`,
  );

  text = replaceRepeated(
    text,
    /\[quote(?:=[^\]]*)?\]([\s\S]*?)\[\/quote\]/gi,
    (_m, body) => `<blockquote>${body}</blockquote>`,
  );

  text = replaceRepeated(
    text,
    /\[spoiler\]([\s\S]*?)\[\/spoiler\]/gi,
    (_m, body) => `<details><summary>Spoiler</summary>${body}</details>`,
  );

  // Heading-style size aliases used by some authors
  text = replaceRepeated(
    text,
    /\[heading\]([\s\S]*?)\[\/heading\]/gi,
    (_m, body) => `<h3>${body}</h3>`,
  );

  // Lists: [list] [*]item [/list] and [list=1]
  text = text.replace(
    /\[list(?:=([^\]]+))?\]([\s\S]*?)\[\/list\]/gi,
    (_m, listType: string | undefined, body: string) => {
      const ordered = listType != null && listType !== "";
      const items = body
        .split(/\[\*\]/i)
        .map((part) => part.trim())
        .filter(Boolean)
        .map((item) => `<li>${item.replace(/\n+$/g, "")}</li>`)
        .join("");
      return ordered ? `<ol>${items}</ol>` : `<ul>${items}</ul>`;
    },
  );

  // Strip unsupported BBCode wrappers but keep inner text (font, style, etc.)
  text = text.replace(
    /\[\/?(?:font|style|table|tr|td|th|line|hr|email|user|mention)(?:=[^\]]*)?\]/gi,
    "",
  );

  // Drop any remaining simple paired unknown tags' markers: [tag]...[/tag] → ...
  text = text.replace(/\[([a-z]+)(?:=[^\]]*)?\]([\s\S]*?)\[\/\1\]/gi, "$2");

  // Remove leftover lone BBCode markers
  text = text.replace(/\[[a-z*/][^[\]]*\]/gi, "");

  return text;
}

/** Minimal Markdown → HTML for collection descriptions. */
export function markdownToHtml(input: string): string {
  let text = escapeHtml(input.replace(/\r\n/g, "\n").replace(/\r/g, "\n"));

  text = text.replace(/```([\s\S]*?)```/g, (_m, body: string) => {
    return `<pre><code>${body.replace(/^\n+|\n+$/g, "")}</code></pre>`;
  });
  text = text.replace(/`([^`]+)`/g, "<code>$1</code>");
  text = text.replace(/^######\s+(.+)$/gm, "<h6>$1</h6>");
  text = text.replace(/^#####\s+(.+)$/gm, "<h5>$1</h5>");
  text = text.replace(/^####\s+(.+)$/gm, "<h4>$1</h4>");
  text = text.replace(/^###\s+(.+)$/gm, "<h3>$1</h3>");
  text = text.replace(/^##\s+(.+)$/gm, "<h2>$1</h2>");
  text = text.replace(/^#\s+(.+)$/gm, "<h1>$1</h1>");
  text = text.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  text = text.replace(/\*([^*]+)\*/g, "<em>$1</em>");
  text = text.replace(
    /\[([^\]]+)\]\((https?:\/\/[^)]+)\)/g,
    (_m, label: string, url: string) => {
      if (!isSafeUrl(url)) return label;
      return `<a href="${url}">${label}</a>`;
    },
  );
  text = text.replace(/^(?:- |\* )(.+)(?:\n(?:- |\* ).+)*/gm, (block) => {
    const items = block
      .split("\n")
      .map((line) => line.replace(/^(?:- |\* )/, "").trim())
      .filter(Boolean)
      .map((item) => `<li>${item}</li>`)
      .join("");
    return `<ul>${items}</ul>`;
  });
  text = text.replace(/\n{2,}/g, "</p><p>");
  text = text.replace(/\n/g, "<br>");
  if (!/^</.test(text)) text = `<p>${text}</p>`;
  return text;
}

export function plainToHtml(input: string): string {
  return escapeHtml(input)
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .replace(/\n{2,}/g, "</p><p>")
    .replace(/\n/g, "<br>")
    .replace(/^(.*)$/s, "<p>$1</p>");
}

export function sanitizeRichHtml(html: string): string {
  ensureHooks();
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS,
    ALLOWED_ATTR,
    ALLOW_DATA_ATTR: false,
  });
}

/**
 * Convert Nexus mod/collection description text (HTML, BBCode, Markdown, or
 * plain) into sanitized HTML suitable for rendering.
 */
export function renderRichText(raw: string | null | undefined): string {
  if (raw == null) return "";
  const trimmed = raw.trim();
  if (!trimmed) return "";

  const hasHtml = looksLikeHtml(trimmed);
  const hasBbcode = looksLikeBbcode(trimmed);

  let html: string;
  if (hasHtml) {
    // REST mod descriptions are often HTML that still embeds BBCode.
    html = hasBbcode ? bbcodeToHtml(trimmed) : trimmed;
  } else if (hasBbcode) {
    html = bbcodeToHtml(escapeHtml(trimmed)).replace(/\n/g, "<br>");
  } else if (/^#{1,6}\s|^\s*[-*]\s|\*\*[^*]+\*\*|\[.+\]\(https?:\/\/.+\)/m.test(trimmed)) {
    html = markdownToHtml(trimmed);
  } else {
    html = plainToHtml(trimmed);
  }

  return sanitizeRichHtml(html);
}
