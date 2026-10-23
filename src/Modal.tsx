import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import { MOTION_EASE, MOTION_FAST } from "./motion";

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

type ModalProps = {
  title: string;
  subtitle?: string;
  className?: string;
  onClose: () => void;
  children: React.ReactNode;
  footer?: React.ReactNode;
};

export function Modal({ title, subtitle, className, onClose, children, footer }: ModalProps) {
  const dialogRef = useRef<HTMLDivElement>(null);
  // Closing must not steal focus from wherever the user was before opening.
  const opener = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const reduced = useReducedMotion() === true;
  const reducedRef = useRef(reduced);
  reducedRef.current = reduced;
  const [open, setOpen] = useState(true);
  const requestCloseRef = useRef(() => {});

  function requestClose() {
    if (reducedRef.current) {
      onCloseRef.current();
      return;
    }
    setOpen(false);
  }
  requestCloseRef.current = requestClose;

  useEffect(() => {
    opener.current = document.activeElement as HTMLElement | null;
    const first = dialogRef.current?.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? dialogRef.current)?.focus();
    return () => opener.current?.focus?.();
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        requestCloseRef.current();
        return;
      }
      if (e.key !== "Tab" || !dialogRef.current) return;
      const focusable = Array.from(
        dialogRef.current.querySelectorAll<HTMLElement>(FOCUSABLE),
      ).filter((el) => el.offsetParent !== null);
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, []);

  useEffect(() => {
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previous;
    };
  }, []);

  const pop = { duration: reduced ? 0 : MOTION_FAST, ease: MOTION_EASE };

  return createPortal(
    <AnimatePresence onExitComplete={() => onCloseRef.current()}>
      {open ? (
        <motion.div
          key="modal"
          className="modal-backdrop"
          initial={reduced ? false : { opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={reduced ? { opacity: 1 } : { opacity: 0 }}
          transition={pop}
          onMouseDown={(e) => e.target === e.currentTarget && requestClose()}
        >
          <motion.div
            className={className ? `modal ${className}` : "modal"}
            role="dialog"
            aria-modal="true"
            aria-label={title}
            tabIndex={-1}
            ref={dialogRef}
            initial={reduced ? false : { opacity: 0, scale: 0.98 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={reduced ? { opacity: 1, scale: 1 } : { opacity: 0, scale: 0.98 }}
            transition={pop}
          >
            <div className="modal-head">
              <div>
                <h2>{title}</h2>
                {subtitle ? <small>{subtitle}</small> : null}
              </div>
              <button type="button" className="linkish" onClick={requestClose} aria-label="Close">
                Close
              </button>
            </div>
            <div className="modal-body">{children}</div>
            {footer ? <div className="modal-foot">{footer}</div> : null}
          </motion.div>
        </motion.div>
      ) : null}
    </AnimatePresence>,
    document.body,
  );
}
