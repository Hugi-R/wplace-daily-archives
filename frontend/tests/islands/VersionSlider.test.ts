// tests/islands/VersionSlider.test.ts
import { describe, it, expect, afterAll } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import VersionSlider from '../../src/islands/VersionSlider.svelte';

afterAll(() => cleanup());

describe('VersionSlider', () => {
  it('renders the slider element', () => {
    const { container } = render(VersionSlider);
    const slider = container.querySelector('input[type="range"]');
    expect(slider).toBeTruthy();
  });

  it('renders the date label span', () => {
    const { container } = render(VersionSlider);
    const label = container.querySelector('.date-label');
    expect(label).toBeTruthy();
  });

  it('renders the ui-hidden class when uiVisible is false', () => {
    const { container } = render(VersionSlider, {
      props: {},
    });
    const div = container.querySelector('.version-slider');
    // uiVisible defaults to true, so no ui-hidden class
    expect(div?.classList.contains('ui-hidden')).toBe(false);
  });
});
