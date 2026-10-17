import { useEffect, useState } from "react";

export type GallerySlide = {
  url: string;
  caption?: string | null;
};

export function ImageCarousel({ slides }: { slides: GallerySlide[] }) {
  const [index, setIndex] = useState(0);
  const slideKey = slides.map((slide) => slide.url).join("\n");

  useEffect(() => {
    setIndex(0);
  }, [slideKey]);

  if (slides.length === 0) return null;

  const current = Math.min(index, slides.length - 1);
  const slide = slides[current];
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
      <div className="detail-carousel-frame">
        <img src={slide.url} alt={slide.caption?.trim() || "Mod image"} />
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
