export type MouseNavDirection = "back" | "forward";

export function isMouseBack(event: MouseEvent): boolean {
  return event.button === 3;
}

export function isMouseForward(event: MouseEvent): boolean {
  return event.button === 4;
}

export function mouseNavDirection(event: MouseEvent): MouseNavDirection | null {
  if (isMouseBack(event)) return "back";
  if (isMouseForward(event)) return "forward";
  return null;
}

export function attachMouseNavigationListeners(
  onNav: (direction: MouseNavDirection, event: MouseEvent) => void,
): () => void {
  const handler = (event: MouseEvent) => {
    const direction = mouseNavDirection(event);
    if (!direction) return;
    event.preventDefault();
    event.stopPropagation();
    onNav(direction, event);
  };
  document.addEventListener("mouseup", handler, true);
  document.addEventListener("auxclick", handler, true);
  return () => {
    document.removeEventListener("mouseup", handler, true);
    document.removeEventListener("auxclick", handler, true);
  };
}
