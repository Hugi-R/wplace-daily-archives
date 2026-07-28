// src/state/stores.ts
import { atom } from 'nanostores';

export interface Viewport {
  lat: number;
  lng: number;
  zoom: number;
}

export const defaultViewport: Viewport = { lat: 0, lng: 0, zoom: 2 };

export const version = atom<number>(0);
export const layer = atom<string>('tiles');
export const viewport = atom<Viewport>(defaultViewport);
export const uiVisible = atom<boolean>(true);
export const locale = atom<string>('en');
export const devMode = atom<boolean>(false);
