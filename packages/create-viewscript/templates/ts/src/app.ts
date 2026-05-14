/**
 * ViewScript Counter Application (Bilayer Architecture)
 *
 * Demonstrates the ViewScript architecture:
 * - WebGPU layer: Visual rendering (panel, buttons)
 * - DOM layer: Interaction + accessibility + text display
 *
 * When .vs parsing is fully implemented, this file will be replaced by:
 *   import { mount } from './main.vs';
 *   const { runtime } = await mount(document.getElementById('app')!);
 */

import { initGpu, createRuntime, type VsRuntime } from '@viewscript/gpu-runtime';
import { getCount, increment, decrement } from './logic';

// =============================================================================
// WebGPU Layer: Mesh Data (compiled output equivalent)
// =============================================================================
// NOTE: This section will be auto-generated from main.vs when the compile
// pipeline is connected. For now, we manually define the mesh geometry.

/**
 * Create vertex data for a rectangle (quad).
 * Vertex layout for solid shader: [x, y, u, v] per vertex.
 */
function createQuadVertices(x: number, y: number, w: number, h: number): Float32Array {
  return new Float32Array([
    // position (x, y), uv (u, v)
    x,     y,     0, 0,  // top-left
    x + w, y,     1, 0,  // top-right
    x + w, y + h, 1, 1,  // bottom-right
    x,     y + h, 0, 1,  // bottom-left
  ]);
}

/** Quad indices (two triangles) */
const QUAD_INDICES = new Uint16Array([0, 1, 2, 2, 3, 0]);

/** Register meshes for the counter UI */
function registerMeshes(runtime: VsRuntime, layout: LayoutMetrics): void {
  // Background panel - Catppuccin Mocha surface0
  runtime.registerMesh('panel', {
    pipelineKey: 'solid',
    vertices: createQuadVertices(layout.panel.x, layout.panel.y, layout.panel.w, layout.panel.h),
    indices: QUAD_INDICES,
    color: [0.12, 0.12, 0.18, 1.0], // #1e1e2e
    positionCount: 8, // 4 vertices * 2 position components
  });

  // Increment button - Catppuccin Mocha green
  runtime.registerMesh('incBtn', {
    pipelineKey: 'solid',
    vertices: createQuadVertices(layout.incBtn.x, layout.incBtn.y, layout.incBtn.w, layout.incBtn.h),
    indices: QUAD_INDICES,
    color: [0.65, 0.89, 0.63, 1.0], // #a6e3a1
    positionCount: 8,
  });

  // Decrement button - Catppuccin Mocha red
  runtime.registerMesh('decBtn', {
    pipelineKey: 'solid',
    vertices: createQuadVertices(layout.decBtn.x, layout.decBtn.y, layout.decBtn.w, layout.decBtn.h),
    indices: QUAD_INDICES,
    color: [0.95, 0.55, 0.66, 1.0], // #f38ba8
    positionCount: 8,
  });
}

/** Update mesh positions when layout changes */
function updateMeshPositions(runtime: VsRuntime, layout: LayoutMetrics): void {
  const { panel, incBtn, decBtn } = layout;

  // Update panel positions (only xy, preserve uv stride)
  runtime.updatePositions('panel', new Float32Array([
    panel.x, panel.y,
    panel.x + panel.w, panel.y,
    panel.x + panel.w, panel.y + panel.h,
    panel.x, panel.y + panel.h,
  ]));

  runtime.updatePositions('incBtn', new Float32Array([
    incBtn.x, incBtn.y,
    incBtn.x + incBtn.w, incBtn.y,
    incBtn.x + incBtn.w, incBtn.y + incBtn.h,
    incBtn.x, incBtn.y + incBtn.h,
  ]));

  runtime.updatePositions('decBtn', new Float32Array([
    decBtn.x, decBtn.y,
    decBtn.x + decBtn.w, decBtn.y,
    decBtn.x + decBtn.w, decBtn.y + decBtn.h,
    decBtn.x, decBtn.y + decBtn.h,
  ]));
}

// =============================================================================
// DOM Layer Setup (Stage 8-10 equivalent)
// =============================================================================

interface DomElements {
  overlay: HTMLDivElement;
  incBtn: HTMLButtonElement;
  decBtn: HTMLButtonElement;
  incLabel: HTMLSpanElement;
  decLabel: HTMLSpanElement;
  countLabel: HTMLSpanElement;
}

function mountDOM(container: HTMLElement): DomElements {
  // Create overlay container
  const overlay = document.createElement('div');
  overlay.style.cssText = 'position:absolute;inset:0;pointer-events:none;overflow:hidden;';
  overlay.setAttribute('aria-live', 'polite');

  // Increment button (transparent click target over WebGPU button)
  const incBtn = document.createElement('button');
  incBtn.style.cssText = 'position:absolute;background:transparent;border:none;cursor:pointer;pointer-events:auto;';
  incBtn.setAttribute('aria-label', 'Increment counter');
  overlay.appendChild(incBtn);

  // Decrement button
  const decBtn = document.createElement('button');
  decBtn.style.cssText = 'position:absolute;background:transparent;border:none;cursor:pointer;pointer-events:auto;';
  decBtn.setAttribute('aria-label', 'Decrement counter');
  overlay.appendChild(decBtn);

  // Button labels (DOM text, visible)
  const incLabel = document.createElement('span');
  incLabel.style.cssText = 'position:absolute;pointer-events:none;color:#1e1e2e;font-family:Inter,system-ui,sans-serif;font-size:32px;font-weight:bold;';
  incLabel.textContent = '+';
  overlay.appendChild(incLabel);

  const decLabel = document.createElement('span');
  decLabel.style.cssText = 'position:absolute;pointer-events:none;color:#1e1e2e;font-family:Inter,system-ui,sans-serif;font-size:32px;font-weight:bold;';
  decLabel.textContent = '-';
  overlay.appendChild(decLabel);

  // Counter display (DOM text, visible)
  const countLabel = document.createElement('span');
  countLabel.style.cssText = 'position:absolute;pointer-events:none;color:#cdd6f4;font-family:Inter,system-ui,sans-serif;font-size:72px;font-weight:bold;text-align:center;';
  countLabel.setAttribute('aria-label', 'Counter value');
  countLabel.textContent = String(getCount());
  overlay.appendChild(countLabel);

  container.appendChild(overlay);
  return { overlay, incBtn, decBtn, incLabel, decLabel, countLabel };
}

function updateDOM(dom: DomElements, layout: LayoutMetrics): void {
  // Position buttons using translate3d (GPU-accelerated)
  dom.incBtn.style.transform = `translate3d(${layout.incBtn.x}px, ${layout.incBtn.y}px, 0)`;
  dom.incBtn.style.width = `${layout.incBtn.w}px`;
  dom.incBtn.style.height = `${layout.incBtn.h}px`;

  dom.decBtn.style.transform = `translate3d(${layout.decBtn.x}px, ${layout.decBtn.y}px, 0)`;
  dom.decBtn.style.width = `${layout.decBtn.w}px`;
  dom.decBtn.style.height = `${layout.decBtn.h}px`;

  // Position button labels (centered on buttons)
  dom.incLabel.style.transform = `translate3d(${layout.incBtn.x + layout.incBtn.w / 2 - 10}px, ${layout.incBtn.y + 8}px, 0)`;
  dom.decLabel.style.transform = `translate3d(${layout.decBtn.x + layout.decBtn.w / 2 - 8}px, ${layout.decBtn.y + 8}px, 0)`;

  // Position counter label (centered on panel)
  dom.countLabel.style.transform = `translate3d(${layout.label.x}px, ${layout.label.y}px, 0)`;
  dom.countLabel.style.width = `${layout.label.w}px`;
}

function bindEvents(
  dom: DomElements,
  runtime: VsRuntime,
  getLayout: () => LayoutMetrics
): void {
  dom.incBtn.addEventListener('click', () => {
    increment();
    dom.countLabel.textContent = String(getCount());
    updateMeshPositions(runtime, getLayout());
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
  });

  dom.decBtn.addEventListener('click', () => {
    decrement();
    dom.countLabel.textContent = String(getCount());
    updateMeshPositions(runtime, getLayout());
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
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
      x: panelX + panelWidth / 2 - 50,
      y: panelY + 40,
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

  // Compute initial layout (CSS pixels)
  let layout = computeLayout(container.clientWidth, container.clientHeight);

  // Register meshes with initial layout
  registerMeshes(runtime, layout);

  // Initialize DOM layer
  const dom = mountDOM(container);
  updateDOM(dom, layout);

  // Bind events with layout getter
  bindEvents(dom, runtime, () => layout);

  // Handle resize
  window.addEventListener('resize', () => {
    layout = computeLayout(container.clientWidth, container.clientHeight);
    updateDOM(dom, layout);
    updateMeshPositions(runtime, layout);
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
    console.log('- WebGPU layer: gpu-runtime (panel, buttons)');
    console.log('- DOM layer: text + interaction');
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
