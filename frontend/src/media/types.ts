export interface ScreenshotConfig {
  layer: string;
  version: number;
  x1: bigint;
  y1: bigint;
  x2: bigint;
  y2: bigint;
}

export interface VideoConfig {
  layer: string;
  x1: bigint;
  y1: bigint;
  x2: bigint;
  y2: bigint;
  from: number;
  to: number;
}
