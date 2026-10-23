import {
  useReducedMotion,
  motion,
  AnimatePresence,
  type HTMLMotionProps,
} from "motion/react";
import {
  type ButtonHTMLAttributes,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";

/** Shared ease. No bounce. */
export const MOTION_EASE = [0.22, 1, 0.36, 1] as const;
/** Controls, dialogs, and panels. */
export const MOTION_FAST = 0.22;
/** Page and view changes. */
export const MOTION_BASE = 0.32;
const STAGGER_CAP = 12;
const STAGGER_STEP = 0.03;

function useReduced() {
  return useReducedMotion() === true;
}

type ViewSwitchProps = {
  viewKey: string;
  children: ReactNode;
  className?: string;
  /** When false, the first paint does not enter. Later key changes still animate. */
  initial?: boolean;
};

/** Crossfades a keyed view. Waits for the exit so two pages never stack. */
export function ViewSwitch({
  viewKey,
  children,
  className,
  initial = true,
}: ViewSwitchProps) {
  const reduced = useReduced();
  return (
    <AnimatePresence mode="wait" initial={initial}>
      <motion.div
        key={viewKey}
        className={["view-enter", className].filter(Boolean).join(" ")}
        initial={reduced ? false : { opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        exit={reduced ? { opacity: 1, y: 0 } : { opacity: 0, y: -6 }}
        transition={{ duration: reduced ? 0 : MOTION_BASE, ease: MOTION_EASE }}
      >
        {children}
      </motion.div>
    </AnimatePresence>
  );
}

type RevealProps = {
  show: boolean;
  className?: string;
  children: ReactNode;
};

/** Fade and a short rise for banners and panels that mount and unmount. */
export function Reveal({ show, className, children }: RevealProps) {
  const reduced = useReduced();
  return (
    <AnimatePresence initial={false}>
      {show ? (
        <motion.div
          key="reveal"
          className={className}
          initial={reduced ? false : { opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={reduced ? { opacity: 1, y: 0 } : { opacity: 0, y: 6 }}
          transition={{ duration: reduced ? 0 : MOTION_FAST, ease: MOTION_EASE }}
        >
          {children}
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}

type PopInProps = {
  show: boolean;
  className?: string;
  children: ReactNode;
  role?: string;
  "aria-multiselectable"?: boolean;
  "aria-labelledby"?: string;
};

/** Opacity and a slight scale for dialogs and popovers. */
export function PopIn({
  show,
  className,
  children,
  role,
  "aria-multiselectable": ariaMultiselectable,
  "aria-labelledby": ariaLabelledBy,
}: PopInProps) {
  const reduced = useReduced();
  return (
    <AnimatePresence initial={false}>
      {show ? (
        <motion.div
          key="pop"
          className={className}
          role={role}
          aria-multiselectable={ariaMultiselectable}
          aria-labelledby={ariaLabelledBy}
          style={{ transformOrigin: "top center" }}
          initial={reduced ? false : { opacity: 0, scale: 0.98 }}
          animate={{ opacity: 1, scale: 1 }}
          exit={reduced ? { opacity: 1, scale: 1 } : { opacity: 0, scale: 0.98 }}
          transition={{ duration: reduced ? 0 : MOTION_FAST, ease: MOTION_EASE }}
        >
          {children}
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}

type MotionCardProps = {
  enterIndex?: number;
  className?: string;
  children: ReactNode;
  onClick?: () => void;
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
  role?: string;
  tabIndex?: number;
};

/** Card arrival. Only the first 12 stagger, so later cards and infinite-scroll appends do not delay. */
export function MotionCard({
  enterIndex,
  className,
  children,
  ...rest
}: MotionCardProps) {
  const reduced = useReduced();
  const stagger = enterIndex != null && enterIndex < STAGGER_CAP && !reduced;
  return (
    <motion.article
      className={className}
      initial={stagger ? { opacity: 0 } : false}
      animate={{ opacity: 1 }}
      transition={{
        duration: stagger ? MOTION_BASE : 0,
        delay: stagger ? enterIndex * STAGGER_STEP : 0,
        ease: MOTION_EASE,
      }}
      {...rest}
    >
      {children}
    </motion.article>
  );
}

/** Fades a block in when it mounts. Remount it to replay, for example after a new search. */
export function FadeIn({
  className,
  style,
  children,
}: {
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
}) {
  const reduced = useReduced();
  return (
    <motion.div
      className={className}
      style={style}
      initial={reduced ? false : { opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: reduced ? 0 : MOTION_BASE, ease: MOTION_EASE }}
    >
      {children}
    </motion.div>
  );
}

function layoutTransition(reduced: boolean): HTMLMotionProps<"span">["transition"] {
  return { layout: { duration: reduced ? 0 : MOTION_FAST, ease: MOTION_EASE } };
}

type HighlightProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  layoutId: string;
  active: boolean;
  indicatorClass: string;
  labelClass: string;
};

function HighlightButton({
  layoutId,
  active,
  indicatorClass,
  labelClass,
  className,
  children,
  type = "button",
  ...rest
}: HighlightProps) {
  const reduced = useReduced();
  return (
    <button type={type} className={className} {...rest}>
      {active ? (
        reduced ? (
          <span className={indicatorClass} aria-hidden="true" />
        ) : (
          <motion.span
            layoutId={layoutId}
            className={indicatorClass}
            aria-hidden="true"
            transition={layoutTransition(reduced)}
          />
        )
      ) : null}
      <span className={labelClass}>{children}</span>
    </button>
  );
}

export function NavTab({
  layoutId,
  active,
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { layoutId: string; active: boolean }) {
  return (
    <HighlightButton
      layoutId={layoutId}
      active={active}
      indicatorClass="nav-indicator"
      labelClass="nav-label"
      className={["nav", active ? "active" : "", className].filter(Boolean).join(" ")}
      {...rest}
    >
      {children}
    </HighlightButton>
  );
}

export function Segment({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return <div className={className ? `segment ${className}` : "segment"}>{children}</div>;
}

export function SegmentTab({
  layoutId,
  active,
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { layoutId: string; active: boolean }) {
  return (
    <HighlightButton
      layoutId={layoutId}
      active={active}
      indicatorClass="segment-indicator"
      labelClass="segment-label"
      className={[active ? "active" : "", className].filter(Boolean).join(" ") || undefined}
      {...rest}
    >
      {children}
    </HighlightButton>
  );
}
