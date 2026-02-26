function isZoomable(el: HTMLElement): el is HTMLImageElement {
  return (
    el.tagName === "IMG" &&
    !!el.closest(".vp-doc") &&
    !el.closest("a") &&
    !/\.svg([?#]|$)/i.test((el as HTMLImageElement).src)
  );
}

export function initImageZoom(): void {
  if (typeof window === "undefined" || (window as any).__imageZoomInit) return;
  (window as any).__imageZoomInit = true;

  let overlay: HTMLDivElement | null = null;

  function close() {
    if (!overlay || overlay.classList.contains("closing")) return;
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
      close();
      return;
    }

    if (!isZoomable(target)) return;

    overlay = document.createElement("div");
    overlay.className = "image-zoom-overlay";

    const img = document.createElement("img");
    img.src = target.src;
    img.className = "image-zoom-img";

    overlay.appendChild(img);
    document.body.appendChild(overlay);
    document.body.style.overflow = "hidden";
  });

  document.addEventListener("keydown", (e: KeyboardEvent) => {
    if (e.key === "Escape") close();
  });

  window.addEventListener("popstate", close);
}
