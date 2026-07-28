// tests/islands/CoordForm.test.ts
import { describe, it, expect, afterAll } from 'vitest';
import { render, cleanup } from '@testing-library/svelte';
import CoordForm from '../../src/islands/CoordForm.svelte';

afterAll(() => cleanup());

describe('CoordForm', () => {
  it('renders with zoom, lat, lng inputs', () => {
    const { container } = render(CoordForm);
    const inputs = container.querySelectorAll('input[type="number"]');
    expect(inputs.length).toBe(3);

    const placeholders = Array.from(inputs).map(
      (input) => input.getAttribute('placeholder')
    );
    expect(placeholders).toContain('Zoom');
    expect(placeholders).toContain('Lat');
    expect(placeholders).toContain('Lng');
  });

  it('renders zoom in and zoom out buttons', () => {
    const { container } = render(CoordForm);
    const zoomButtons = container.querySelectorAll('.zoom-buttons button');
    expect(zoomButtons.length).toBe(2);
  });
});
