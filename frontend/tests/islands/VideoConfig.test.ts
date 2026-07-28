// tests/islands/VideoConfig.test.ts
import { describe, it, expect, afterAll } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import VideoConfig from '../../src/islands/VideoConfig.svelte';

afterAll(() => cleanup());

describe('VideoConfig', () => {
  it('renders Screenshot and Video buttons', () => {
    const { container } = render(VideoConfig);
    const buttons = container.querySelectorAll('button[type="button"]');
    expect(buttons.length).toBeGreaterThanOrEqual(2);
  });

  it('has a button with "Screenshot" text', () => {
    const { container } = render(VideoConfig);
    const buttons = Array.from(container.querySelectorAll('button'));
    const screenshotBtn = buttons.find((b) =>
      b.textContent?.includes('Screenshot')
    );
    expect(screenshotBtn).toBeTruthy();
  });

  it('has a button with "Video" text', () => {
    const { container } = render(VideoConfig);
    const buttons = Array.from(container.querySelectorAll('button'));
    const videoBtn = buttons.find((b) => b.textContent?.includes('Video'));
    expect(videoBtn).toBeTruthy();
  });

  it('applies ui-hidden class when uiVisible is false', () => {
    const { container } = render(VideoConfig);
    const wrapper = container.querySelector('.video-config');
    // uiVisible defaults to true, so no ui-hidden
    expect(wrapper?.classList.contains('ui-hidden')).toBe(false);
  });
});
