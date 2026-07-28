// tests/state/stores.test.ts
import { describe, it, expect, beforeEach } from 'vitest';
import {
  version,
  layer,
  viewport,
  uiVisible,
  devMode,
  defaultViewport,
} from '../../src/state/stores';

describe('stores', () => {
  beforeEach(() => {
    version.set(0);
    layer.set('tiles');
    viewport.set(defaultViewport);
    uiVisible.set(true);
    devMode.set(false);
  });

  it('version defaults to 0', () => {
    expect(version.get()).toBe(0);
  });

  it('layer defaults to "tiles"', () => {
    expect(layer.get()).toBe('tiles');
  });

  it('viewport defaults to center [0,0] zoom 2', () => {
    expect(viewport.get()).toEqual({ lat: 0, lng: 0, zoom: 2 });
  });

  it('uiVisible defaults to true', () => {
    expect(uiVisible.get()).toBe(true);
  });

  it('devMode defaults to false', () => {
    expect(devMode.get()).toBe(false);
  });

  it('stores are independently settable', () => {
    version.set(42);
    layer.set('alternate');
    viewport.set({ lat: 40.7, lng: -74.0, zoom: 5 });
    uiVisible.set(false);
    devMode.set(true);

    expect(version.get()).toBe(42);
    expect(layer.get()).toBe('alternate');
    expect(viewport.get()).toEqual({ lat: 40.7, lng: -74.0, zoom: 5 });
    expect(uiVisible.get()).toBe(false);
    expect(devMode.get()).toBe(true);
  });
});
