import { useEffect, useRef, useState, type CSSProperties } from "react";

export type GallerySlide = {
  url: string;
  caption?: string | null;
};

function readRatio(img: HTMLImageElement): number | null {
  if (img.naturalWidth > 0 && img.naturalHeight > 0) {
    return img.naturalWidth / img.naturalHeight;
  }
  return null;
}

export function ImageCarousel({ slides }: { slides: GallerySlide[] }) {
  const [index, setIndex] = useState(0);
  const [ratio, setRatio] = useState<number | null>(null);
  const imgRef = useRef<HTMLImageElement>(null);
  const slideKey = slides.map((slide) => slide.url).join("\n");
  const current = slides.length === 0 ? 0 : Math.min(index, slides.length - 1);
  const slide = slides[current];

  useEffect(() => {
    setIndex(0);
  }, [slideKey]);

  useEffect(() => {
    const img = imgRef.current;
    const next = img?.complete ? readRatio(img) : null;
    setRatio(next);
  }, [slide?.url]);

  if (!slide) return null;

  const multiple = slides.length > 1;

  function step(delta: number) {
    setIndex((i) => {
      const next = i + delta;
      const count = slides.length;
      return ((next % count) + count) % count;
    });
  }

  return (
    <div
      className="detail-carousel"
      tabIndex={0}
      role="region"
      aria-roledescription="carousel"
      aria-label="Mod images"
      onKeyDown={(event) => {
        if (!multiple) return;
        if (event.key === "ArrowLeft") {
          event.preventDefault();
          step(-1);
        } else if (event.key === "ArrowRight") {
          event.preventDefault();
          step(1);
        }
      }}
    >
      <div
        className="detail-carousel-frame"
        style={
          ratio != null
            ? ({ "--slide-ratio": String(ratio) } as CSSProperties)
            : undefined
        }
      >
        <img
          key={slide.url}
          ref={imgRef}
          src={slide.url}
          alt={slide.caption?.trim() || "Mod image"}
          onLoad={(event) => {
            const img = event.currentTarget;
            if (img.getAttribute("src") !== slide.url) return;
            const next = readRatio(img);
            if (next != null) setRatio(next);
          }}
        />
        {multiple ? (
          <>
            <button
              type="button"
              className="detail-carousel-prev"
              aria-label="Previous image"
              onClick={() => step(-1)}
            >
              ‹
            </button>
            <button
              type="button"
              className="detail-carousel-next"
              aria-label="Next image"
              onClick={() => step(1)}
            >
              ›
            </button>
            <span className="detail-carousel-count" aria-live="polite">
              {current + 1} / {slides.length}
            </span>
          </>
        ) : null}
      </div>
      {slide.caption?.trim() ? (
        <p className="detail-carousel-caption">{slide.caption}</p>
      ) : null}
    </div>
  );
}
