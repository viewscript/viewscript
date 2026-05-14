/**
 * ViewScript Counter Application (Bilayer Architecture)
 *
 * Demonstrates the ViewScript architecture:
 * - WebGPU layer: Visual rendering (gpu-runtime)
 * - DOM layer: Interaction + accessibility (transparent overlay)
 *
 * When .vs parsing is fully implemented, this file will be replaced by:
 *   import { mount } from './main.vs';
 *   const { runtime } = await mount(document.getElementById('app')!);
 */

import { initGpu, createRuntime, type VsRuntime } from '@viewscript/gpu-runtime';
import { getCount, increment, decrement } from './logic';

// =============================================================================
// DOM Layer Setup (Stage 8-10 equivalent)
// =============================================================================

interface DomElements {
  overlay: HTMLDivElement;
  incBtn: HTMLButtonElement;
  decBtn: HTMLButtonElement;
  label: HTMLSpanElement;
}

function mountDOM(container: HTMLElement): DomElements {
  // Create overlay container
  const overlay = document.createElement('div');
  overlay.style.cssText = 'position:absolute;inset:0;pointer-events:none;overflow:hidden;';
  overlay.setAttribute('aria-live', 'polite');

  // Increment button (transparent, positioned over WebGPU button)
  const incBtn = document.createElement('button');
  incBtn.style.cssText = 'position:absolute;background:transparent;border:none;cursor:pointer;pointer-events:auto;';
  incBtn.setAttribute('aria-label', 'Increment counter');
  overlay.appendChild(incBtn);

  // Decrement button
  const decBtn = document.createElement('button');
  decBtn.style.cssText = 'position:absolute;background:transparent;border:none;cursor:pointer;pointer-events:auto;';
  decBtn.setAttribute('aria-label', 'Decrement counter');
  overlay.appendChild(decBtn);

  // Counter label (for screen readers)
  const label = document.createElement('span');
  label.style.cssText = 'position:absolute;background:transparent;pointer-events:none;color:transparent;';
  label.setAttribute('aria-label', 'Counter value');
  label.textContent = String(getCount());
  overlay.appendChild(label);

  container.appendChild(overlay);
  return { overlay, incBtn, decBtn, label };
}

function updateDOM(dom: DomElements, layout: LayoutMetrics): void {
  // Position buttons using translate3d (GPU-accelerated)
  dom.incBtn.style.transform = `translate3d(${layout.incBtn.x}px, ${layout.incBtn.y}px, 0)`;
  dom.incBtn.style.width = `${layout.incBtn.w}px`;
  dom.incBtn.style.height = `${layout.incBtn.h}px`;

  dom.decBtn.style.transform = `translate3d(${layout.decBtn.x}px, ${layout.decBtn.y}px, 0)`;
  dom.decBtn.style.width = `${layout.decBtn.w}px`;
  dom.decBtn.style.height = `${layout.decBtn.h}px`;

  dom.label.style.transform = `translate3d(${layout.label.x}px, ${layout.label.y}px, 0)`;
}

function bindEvents(
  dom: DomElements,
  runtime: VsRuntime,
  onUpdate: () => void
): void {
  dom.incBtn.addEventListener('click', () => {
    increment();
    dom.label.textContent = String(getCount());
    onUpdate();
    // Note: In compiled output, render() is called automatically
  });

  dom.decBtn.addEventListener('click', () => {
    decrement();
    dom.label.textContent = String(getCount());
    onUpdate();
  });
}

// =============================================================================
// Layout Metrics (P-dimension resolved values)
// =============================================================================

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface LayoutMetrics {
  panel: Rect;
  incBtn: Rect;
  decBtn: Rect;
  label: Rect;
}

function computeLayout(viewportWidth: number, viewportHeight: number): LayoutMetrics {
  const panelWidth = 320;
  const panelHeight = 200;
  const panelX = (viewportWidth - panelWidth) / 2;
  const panelY = (viewportHeight - panelHeight) / 2;

  const btnWidth = 80;
  const btnHeight = 48;
  const btnY = panelY + panelHeight - btnHeight - 24;
  const btnSpacing = 40;

  return {
    panel: { x: panelX, y: panelY, w: panelWidth, h: panelHeight },
    incBtn: {
      x: panelX + panelWidth - btnSpacing - btnWidth,
      y: btnY,
      w: btnWidth,
      h: btnHeight,
    },
    decBtn: {
      x: panelX + btnSpacing,
      y: btnY,
      w: btnWidth,
      h: btnHeight,
    },
    label: {
      x: panelX + panelWidth / 2 - 20,
      y: panelY + 50,
      w: 100,
      h: 80,
    },
  };
}

// =============================================================================
// Main Entry Point (Stage 11 equivalent)
// =============================================================================

async function mount(container: HTMLElement): Promise<{
  runtime: VsRuntime;
  canvas: HTMLCanvasElement;
  overlay: HTMLDivElement;
}> {
  // Create canvas for WebGPU layer
  const canvas = document.createElement('canvas');
  canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;';
  container.style.position = 'relative';
  container.appendChild(canvas);

  // Match canvas backing store to container (DPR-scaled)
  const resizeCanvas = (): void => {
    const rect = container.getBoundingClientRect();
    canvas.width = rect.width * devicePixelRatio;
    canvas.height = rect.height * devicePixelRatio;
  };
  resizeCanvas();
  window.addEventListener('resize', resizeCanvas);

  // Initialize WebGPU layer
  const gpu = await initGpu(canvas);
  const runtime = createRuntime(gpu);

  // Initialize DOM layer
  const dom = mountDOM(container);

  // Compute initial layout (CSS pixels)
  let layout = computeLayout(container.clientWidth, container.clientHeight);
  updateDOM(dom, layout);

  // Bind events with render callback
  bindEvents(dom, runtime, () => {
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 }); // #11111b
  });

  // Handle resize
  window.addEventListener('resize', () => {
    layout = computeLayout(container.clientWidth, container.clientHeight);
    updateDOM(dom, layout);
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
  });

  // Initial render
  runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });

  return { runtime, canvas, overlay: dom.overlay };
}

// =============================================================================
// Application Bootstrap
// =============================================================================

const container = document.getElementById('app');
if (!container) {
  throw new Error('Container element #app not found');
}

mount(container)
  .then(({ runtime }) => {
    console.log('ViewScript mounted (bilayer architecture)');
    console.log('- WebGPU layer: gpu-runtime');
    console.log('- DOM layer: transparent overlay');
  })
  .catch((error) => {
    console.error('ViewScript initialization failed:', error);
    document.body.innerHTML = `
      <div style="color: #f38ba8; padding: 40px; font-family: monospace;">
        <h2>ViewScript Error</h2>
        <pre>${error}</pre>
        <p style="color: #a6adc8; margin-top: 20px;">
          Check the browser console for details.<br>
          WebGPU requires Chrome 113+, Edge 113+, or Firefox Nightly.
        </p>
      </div>
    `;
  });
