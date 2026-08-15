import type { ReactNode } from "react";
import { renderRichText } from "./richText";

type RichTextProps = {
  text: string | null | undefined;
  className?: string;
  empty?: ReactNode;
};

/** Renders Nexus HTML / BBCode / Markdown descriptions as sanitized rich text. */
export function RichText({ text, className, empty = "No description available." }: RichTextProps) {
  const html = renderRichText(text);
  if (!html) {
    return <p className={className}>{empty}</p>;
  }
  return (
    <div
      className={className}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
