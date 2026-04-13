const slides = Array.from(document.querySelectorAll(".slide"));
const prevButton = document.getElementById("prev-button");
const nextButton = document.getElementById("next-button");
const slideLabel = document.getElementById("slide-label");
const slideCount = document.getElementById("slide-count");
const progressBar = document.getElementById("progress-bar");

let currentIndex = 0;
let touchStartX = 0;

function indexFromHash() {
  const hash = window.location.hash.replace("#slide-", "");
  const parsed = Number.parseInt(hash, 10);
  if (Number.isNaN(parsed) || parsed < 1 || parsed > slides.length) {
    return 0;
  }
  return parsed - 1;
}

function updateSlide(index, replaceHash = false) {
  currentIndex = Math.max(0, Math.min(index, slides.length - 1));

  slides.forEach((slide, slideIndex) => {
    const active = slideIndex === currentIndex;
    slide.classList.toggle("active", active);
    slide.toggleAttribute("hidden", !active);
  });

  const activeSlide = slides[currentIndex];
  slideLabel.textContent = `${String(currentIndex + 1).padStart(2, "0")}. ${activeSlide.dataset.title}`;
  slideCount.textContent = `${currentIndex + 1} / ${slides.length}`;
  progressBar.style.width = `${((currentIndex + 1) / slides.length) * 100}%`;

  prevButton.disabled = currentIndex === 0;
  nextButton.disabled = currentIndex === slides.length - 1;

  const nextHash = `#slide-${currentIndex + 1}`;
  if (replaceHash) {
    history.replaceState(null, "", nextHash);
  } else if (window.location.hash !== nextHash) {
    history.pushState(null, "", nextHash);
  }
}

function move(step) {
  updateSlide(currentIndex + step);
}

prevButton.addEventListener("click", () => move(-1));
nextButton.addEventListener("click", () => move(1));

window.addEventListener("keydown", (event) => {
  if (event.key === "ArrowRight" || event.key === "PageDown" || event.key === " ") {
    event.preventDefault();
    move(1);
  }

  if (event.key === "ArrowLeft" || event.key === "PageUp") {
    event.preventDefault();
    move(-1);
  }

  if (event.key === "Home") {
    event.preventDefault();
    updateSlide(0);
  }

  if (event.key === "End") {
    event.preventDefault();
    updateSlide(slides.length - 1);
  }
});

window.addEventListener("hashchange", () => updateSlide(indexFromHash(), true));

document.addEventListener("touchstart", (event) => {
  touchStartX = event.changedTouches[0].screenX;
});

document.addEventListener("touchend", (event) => {
  const delta = event.changedTouches[0].screenX - touchStartX;
  if (Math.abs(delta) < 48) {
    return;
  }
  move(delta < 0 ? 1 : -1);
});

updateSlide(indexFromHash(), true);
