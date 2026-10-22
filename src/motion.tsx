import type { ReactNode } from "react";

/** Layout wrapper for a tab panel. */
export function ViewSlot({ children }: { children: ReactNode }) {
  return <div className="view-slot">{children}</div>;
}
