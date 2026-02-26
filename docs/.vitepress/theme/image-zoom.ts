let initialized = false;

function isZoomable(el: HTMLElement): el is HTMLImageElement {
  return (
    el.tagName === "IMG" &&
    !!el.closest(".vp-doc") &&
    !el.closest("a") &&
    !(el as HTMLImageElement).src.endsWith(".svg")
  );
}

export function initImageZoom(): void {
  if (typeof window === "undefined" || initialized) return;
  initialized = true;

  let overlay: HTMLDivElement | null = null;

  function close() {
    if (!overlay) return;
    overlay.classList.add("closing");
    overlay.addEventListener(
      "animationend",
      () => {
        overlay?.remove();
        overlay = null;
      },
      { once: true },
    );
    document.body.style.overflow = "";
  }

  document.addEventListener("click", (e: MouseEvent) => {
    const target = e.target as HTMLElement;

    if (overlay) {
      if (target.closest(".image-zoom-img")) return;
      close();
      return;
    }

    if (!isZoomable(target)) return;

    overlay = document.createElement("div");
    overlay.className = "image-zoom-overlay";

    const img = document.createElement("img");
    img.src = target.src;
    if (target.srcset) img.srcset = target.srcset;
    img.className = "image-zoom-img";

    overlay.appendChild(img);
    document.body.appendChild(overlay);
    document.body.style.overflow = "hidden";
  });

  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
  });
}
