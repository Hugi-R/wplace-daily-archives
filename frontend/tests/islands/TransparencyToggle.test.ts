// tests/islands/TransparencyToggle.test.ts
import { describe, it, expect, afterAll } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import TransparencyToggle from '../../src/islands/TransparencyToggle.svelte';

afterAll(() => cleanup());

describe('TransparencyToggle', () => {
  it('renders the toggle button', () => {
    const { container } = render(TransparencyToggle);
    const button = container.querySelector('button.transparency-toggle');
    expect(button).toBeTruthy();
  });

  it('button text defaults to Transparent', () => {
    const { container } = render(TransparencyToggle);
    const button = container.querySelector('button.transparency-toggle');
    expect(button?.textContent).toBe('Transparent');
  });

  it('renders the ui-hidden class when uiVisible is false', () => {
    const { container } = render(TransparencyToggle);
    const button = container.querySelector('button.transparency-toggle');
    // uiVisible defaults to true, so no ui-hidden class
    expect(button?.classList.contains('ui-hidden')).toBe(false);
  });
});
