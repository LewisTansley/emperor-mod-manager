import { beforeEach, describe, expect, it } from "vitest";
import {
  bbcodeToHtml,
  escapeHtml,
  isSafeColor,
  isSafeUrl,
  looksLikeBbcode,
  looksLikeHtml,
  renderRichText,
  sanitizeRichHtml,
} from "./richTextFormat";

describe("escapeHtml", () => {
  it("escapes markup characters", () => {
    expect(escapeHtml(`<script>"x"&'y'</script>`)).toBe(
      "&lt;script&gt;&quot;x&quot;&amp;&#39;y&#39;&lt;/script&gt;",
    );
  });
});

describe("looksLike helpers", () => {
  it("detects html and bbcode", () => {
    expect(looksLikeHtml("<p>hi</p>")).toBe(true);
    expect(looksLikeHtml("[b]hi[/b]")).toBe(false);
    expect(looksLikeBbcode("[size=5][b]hi[/b][/size]")).toBe(true);
    expect(looksLikeBbcode("plain text")).toBe(false);
  });
});

describe("url/color safety", () => {
  it("allows only http(s) urls", () => {
    expect(isSafeUrl("https://example.com/a")).toBe(true);
    expect(isSafeUrl("http://example.com")).toBe(true);
    expect(isSafeUrl("javascript:alert(1)")).toBe(false);
    expect(isSafeUrl("data:text/html,hi")).toBe(false);
  });

  it("allows hex and named colors only", () => {
    expect(isSafeColor("#ffa500")).toBe(true);
    expect(isSafeColor("#fff")).toBe(true);
    expect(isSafeColor("orange")).toBe(true);
    expect(isSafeColor("expression(alert(1))")).toBe(false);
    expect(isSafeColor("red; background:url(x)")).toBe(false);
  });
});

describe("bbcodeToHtml", () => {
  it("renders nested size/color/bold", () => {
    const html = bbcodeToHtml(
      "[size=5][b][color=#ffa500]Newest Version[/color][/b][/size]",
    );
    expect(html).toContain("<strong>");
    expect(html).toContain('style="color: #ffa500"');
    expect(html).toContain('style="font-size: 1.375em"');
    expect(html).toContain("Newest Version");
    expect(html).not.toContain("[size=");
    expect(html).not.toContain("[b]");
  });

  it("renders hyperlinks and youtube as safe links", () => {
    expect(bbcodeToHtml("[url=https://example.com/x]Click[/url]")).toBe(
      '<a href="https://example.com/x">Click</a>',
    );
    expect(bbcodeToHtml("[url]https://example.com/y[/url]")).toBe(
      '<a href="https://example.com/y">https://example.com/y</a>',
    );
    expect(bbcodeToHtml("[youtube]9DXBti-c6C0[/youtube]")).toBe(
      '<a href="https://www.youtube.com/watch?v=9DXBti-c6C0">YouTube: 9DXBti-c6C0</a>',
    );
    expect(bbcodeToHtml("[url=javascript:alert(1)]x[/url]")).toBe("x");
  });

  it("renders lists and line-oriented content", () => {
    const html = bbcodeToHtml("[list]\n[*]One\n[*]Two\n[/list]");
    expect(html).toContain("<ul>");
    expect(html).toContain("<li>One</li>");
    expect(html).toContain("<li>Two</li>");
    const ordered = bbcodeToHtml("[list=1][*]A[*]B[/list]");
    expect(ordered).toContain("<ol>");
  });

  it("strips unsupported wrappers instead of showing them", () => {
    expect(bbcodeToHtml("[font=Arial]Hello[/font]")).toBe("Hello");
    expect(bbcodeToHtml("[unknown]Keep[/unknown]")).toBe("Keep");
  });

  it("converts bbcode embedded inside html", () => {
    const html = bbcodeToHtml("<p>[b]Bold[/b] and [color=red]R[/color]</p>");
    expect(html).toBe('<p><strong>Bold</strong> and <span style="color: red">R</span></p>');
  });
});

describe("sanitizeRichHtml", () => {
  beforeEach(() => {
    // Ensure DOM is available for DOMPurify (vitest jsdom).
    expect(typeof window).toBe("object");
  });

  it("removes scripts and unsafe urls", () => {
    const dirty =
      '<p>ok</p><script>alert(1)</script><a href="javascript:alert(1)">x</a><img src="https://cdn.example/a.png">';
    const clean = sanitizeRichHtml(dirty);
    expect(clean).toContain("<p>ok</p>");
    expect(clean).not.toContain("script");
    expect(clean).not.toContain("javascript:");
    expect(clean).toContain('src="https://cdn.example/a.png"');
  });

  it("keeps safe inline color/size styles only", () => {
    const clean = sanitizeRichHtml(
      '<span style="color: #ffa500; font-size: 1.375em; background: url(x)">Hi</span>',
    );
    expect(clean).toContain("color: #ffa500");
    expect(clean).toContain("font-size: 1.375em");
    expect(clean).not.toContain("background");
  });
});

describe("renderRichText", () => {
  it("renders the screenshot-like bbcode payload", () => {
    const raw =
      "[size=5][b][color=#ffa500]Newest Version 2.11 brings over 200 new Facial Expressions thanks to new Collab![/color][/b][/size]\n\n[b][color=#ffa500][size=3]Warning about Vortex[/size][/color][/b]\n[youtube]9DXBti-c6C0[/youtube]";
    const html = renderRichText(raw);
    expect(html).toContain("Newest Version 2.11");
    expect(html).toContain("Facial Expressions");
    expect(html).toContain("youtube.com/watch?v=9DXBti-c6C0");
    expect(html).not.toContain("[size=");
    expect(html).not.toContain("[youtube]");
    expect(html).not.toContain("[b]");
  });

  it("sanitizes html descriptions", () => {
    const html = renderRichText('<p>Hello <a href="https://nexusmods.com">Nexus</a></p>');
    expect(html).toContain("<p>");
    expect(html).toContain('href="https://nexusmods.com"');
    expect(html).toContain('rel="noopener noreferrer"');
  });

  it("returns empty for blank input", () => {
    expect(renderRichText("")).toBe("");
    expect(renderRichText("   ")).toBe("");
    expect(renderRichText(null)).toBe("");
  });
});
