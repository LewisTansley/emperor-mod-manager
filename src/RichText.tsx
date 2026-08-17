import { openUrl } from "@tauri-apps/plugin-opener";
import type { MouseEvent, ReactNode } from "react";
import { isSafeUrl, renderRichText } from "./richTextFormat";

type RichTextProps = {
  text: string | null | undefined;
  className?: string;
  empty?: ReactNode;
};

function onRichTextClick(e: MouseEvent<HTMLDivElement>) {
  const target = e.target;
  if (!(target instanceof Element)) return;
  const anchor = target.closest("a");
  if (!anchor || !e.currentTarget.contains(anchor)) return;
  const href = anchor.getAttribute("href");
  if (!href || !isSafeUrl(href)) return;
  e.preventDefault();
  e.stopPropagation();
  void openUrl(href).catch((err) => {
    console.error("Failed to open URL", err);
  });
}

/** Renders Nexus HTML / BBCode / Markdown descriptions as sanitized rich text. */
export function RichText({ text, className, empty = "No description available." }: RichTextProps) {
  const html = renderRichText(text);
  if (!html) {
    return <p className={className}>{empty}</p>;
  }
  return (
    <div
      className={className}
      onClick={onRichTextClick}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
