import type { Container } from "pixi.js";
import type { SemanticZoomLevel } from "./game-types";

interface CameraOptions {
  canvas: HTMLCanvasElement;
  world: Container;
  worldWidth: number;
  worldHeight: number;
  onChange: (zoomLevel: SemanticZoomLevel, userZoom: number) => void;
}

interface PointerPosition {
  x: number;
  y: number;
}

const clamp = (value: number, min: number, max: number): number =>
  Math.min(Math.max(value, min), max);

export class SceneCameraController {
  private readonly canvas: HTMLCanvasElement;
  private readonly world: Container;
  private readonly worldWidth: number;
  private readonly worldHeight: number;
  private readonly onChange: (zoomLevel: SemanticZoomLevel, userZoom: number) => void;
  private readonly pointers = new Map<number, PointerPosition>();
  private viewportWidth = 1;
  private viewportHeight = 1;
  private fitScale = 1;
  private userZoom = 1;
  private x = 0;
  private y = 0;
  private previousCentroid: PointerPosition | null = null;
  private previousDistance = 0;

  constructor(options: CameraOptions) {
    this.canvas = options.canvas;
    this.world = options.world;
    this.worldWidth = options.worldWidth;
    this.worldHeight = options.worldHeight;
    this.onChange = options.onChange;
    this.canvas.addEventListener("wheel", this.onWheel, { passive: false });
    this.canvas.addEventListener("pointerdown", this.onPointerDown);
    this.canvas.addEventListener("pointermove", this.onPointerMove);
    this.canvas.addEventListener("pointerup", this.onPointerUp);
    this.canvas.addEventListener("pointercancel", this.onPointerUp);
  }

  resize(width: number, height: number): void {
    const previousFit = this.fitScale;
    const worldCenter = this.screenToWorld(this.viewportWidth / 2, this.viewportHeight / 2);
    this.viewportWidth = Math.max(1, width);
    this.viewportHeight = Math.max(1, height);
    this.fitScale = Math.min(
      this.viewportWidth / this.worldWidth,
      this.viewportHeight / this.worldHeight
    );
    if (previousFit <= 0 || !Number.isFinite(worldCenter.x) || !Number.isFinite(worldCenter.y)) {
      this.reset();
      return;
    }
    const scale = this.scale();
    this.x = this.viewportWidth / 2 - worldCenter.x * scale;
    this.y = this.viewportHeight / 2 - worldCenter.y * scale;
    this.apply();
  }

  reset(): void {
    this.userZoom = 1;
    const scale = this.scale();
    this.x = (this.viewportWidth - this.worldWidth * scale) / 2;
    this.y = (this.viewportHeight - this.worldHeight * scale) / 2;
    this.apply();
  }

  zoomBy(factor: number): void {
    this.zoomAt(
      this.userZoom * factor,
      this.viewportWidth / 2,
      this.viewportHeight / 2
    );
  }

  currentZoom(): number {
    return this.userZoom;
  }

  zoomLevel(): SemanticZoomLevel {
    if (this.userZoom < 1.18) return "overview";
    if (this.userZoom < 1.82) return "operations";
    return "detail";
  }

  destroy(): void {
    this.canvas.removeEventListener("wheel", this.onWheel);
    this.canvas.removeEventListener("pointerdown", this.onPointerDown);
    this.canvas.removeEventListener("pointermove", this.onPointerMove);
    this.canvas.removeEventListener("pointerup", this.onPointerUp);
    this.canvas.removeEventListener("pointercancel", this.onPointerUp);
    this.pointers.clear();
  }

  private scale(): number {
    return this.fitScale * this.userZoom;
  }

  private screenToWorld(screenX: number, screenY: number): PointerPosition {
    const scale = this.scale();
    return {
      x: (screenX - this.x) / scale,
      y: (screenY - this.y) / scale,
    };
  }

  private zoomAt(nextUserZoom: number, screenX: number, screenY: number): void {
    const worldPoint = this.screenToWorld(screenX, screenY);
    this.userZoom = clamp(nextUserZoom, 0.92, 2.8);
    const scale = this.scale();
    this.x = screenX - worldPoint.x * scale;
    this.y = screenY - worldPoint.y * scale;
    this.apply();
  }

  private apply(): void {
    const scale = this.scale();
    const scaledWidth = this.worldWidth * scale;
    const scaledHeight = this.worldHeight * scale;
    const padding = Math.min(this.viewportWidth, this.viewportHeight) * 0.06;

    this.x =
      scaledWidth <= this.viewportWidth
        ? (this.viewportWidth - scaledWidth) / 2
        : clamp(this.x, this.viewportWidth - scaledWidth - padding, padding);
    this.y =
      scaledHeight <= this.viewportHeight
        ? (this.viewportHeight - scaledHeight) / 2
        : clamp(this.y, this.viewportHeight - scaledHeight - padding, padding);

    this.world.position.set(this.x, this.y);
    this.world.scale.set(scale);
    this.onChange(this.zoomLevel(), this.userZoom);
  }

  private localPointer(event: MouseEvent): PointerPosition {
    const bounds = this.canvas.getBoundingClientRect();
    return {
      x: ((event.clientX - bounds.left) / Math.max(bounds.width, 1)) * this.viewportWidth,
      y: ((event.clientY - bounds.top) / Math.max(bounds.height, 1)) * this.viewportHeight,
    };
  }

  private pointerGeometry(): { centroid: PointerPosition; distance: number } {
    const positions = [...this.pointers.values()];
    const first = positions[0] ?? { x: 0, y: 0 };
    const second = positions[1] ?? first;
    return {
      centroid: {
        x: (first.x + second.x) / 2,
        y: (first.y + second.y) / 2,
      },
      distance: Math.hypot(second.x - first.x, second.y - first.y),
    };
  }

  private readonly onWheel = (event: WheelEvent): void => {
    event.preventDefault();
    const point = this.localPointer(event);
    this.zoomAt(this.userZoom * Math.exp(-event.deltaY * 0.0014), point.x, point.y);
  };

  private readonly onPointerDown = (event: PointerEvent): void => {
    if (event.button !== 0) return;
    this.canvas.setPointerCapture(event.pointerId);
    this.pointers.set(event.pointerId, this.localPointer(event));
    const geometry = this.pointerGeometry();
    this.previousCentroid = geometry.centroid;
    this.previousDistance = geometry.distance;
  };

  private readonly onPointerMove = (event: PointerEvent): void => {
    if (!this.pointers.has(event.pointerId)) return;
    this.pointers.set(event.pointerId, this.localPointer(event));
    const geometry = this.pointerGeometry();
    if (this.previousCentroid) {
      this.x += geometry.centroid.x - this.previousCentroid.x;
      this.y += geometry.centroid.y - this.previousCentroid.y;
    }
    if (this.pointers.size >= 2 && this.previousDistance > 0 && geometry.distance > 0) {
      const ratio = geometry.distance / this.previousDistance;
      const worldPoint = this.screenToWorld(geometry.centroid.x, geometry.centroid.y);
      this.userZoom = clamp(this.userZoom * ratio, 0.92, 2.8);
      const scale = this.scale();
      this.x = geometry.centroid.x - worldPoint.x * scale;
      this.y = geometry.centroid.y - worldPoint.y * scale;
    }
    this.previousCentroid = geometry.centroid;
    this.previousDistance = geometry.distance;
    this.apply();
  };

  private readonly onPointerUp = (event: PointerEvent): void => {
    this.pointers.delete(event.pointerId);
    if (this.canvas.hasPointerCapture(event.pointerId)) {
      this.canvas.releasePointerCapture(event.pointerId);
    }
    if (this.pointers.size === 0) {
      this.previousCentroid = null;
      this.previousDistance = 0;
      return;
    }
    const geometry = this.pointerGeometry();
    this.previousCentroid = geometry.centroid;
    this.previousDistance = geometry.distance;
  };
}
