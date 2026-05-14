/**
 * ViewScript Application Runtime
 *
 * Initializes the WASM engine and sets up the render loop.
 * You typically don't need to modify this file.
 */

import init, { WasmViewScriptEngine } from '@viewscript/wasm';

// Canvas setup
const canvas = document.getElementById('canvas') as HTMLCanvasElement;
const dpr = window.devicePixelRatio || 1;

// Resize canvas to fill viewport
function resizeCanvas(): void {
  canvas.width = window.innerWidth * dpr;
  canvas.height = window.innerHeight * dpr;
  canvas.style.width = '100vw';
  canvas.style.height = '100vh';
}

// Build Q-dimension snapshot for current frame
function buildQSnapshot(): object {
  return {
    values: {
      'input.pointer.x': { type: 'Float', value: pointerState.x },
      'input.pointer.y': { type: 'Float', value: pointerState.y },
      'input.pointer.pressed': { type: 'Bool', value: pointerState.pressed },
      'env.viewport.width': { type: 'Int', value: canvas.width },
      'env.viewport.height': { type: 'Int', value: canvas.height },
      'env.viewport.dpr': { type: 'Float', value: dpr },
    },
  };
}

// Pointer state tracking
const pointerState = {
  x: 0,
  y: 0,
  pressed: false,
};

// Main initialization
async function main(): Promise<void> {
  // Initialize WASM
  await init();

  // Setup canvas
  resizeCanvas();
  document.body.style.margin = '0';
  document.body.style.background = '#1a1a2e';

  // Create ViewScript engine
  const engine = await WasmViewScriptEngine.create(canvas, dpr);
  console.log('ViewScript engine initialized:', canvas.width, 'x', canvas.height);

  // Add demo component (matches main.vs HelloBox definition)
  // TODO: Replace with .vs parser when available
  engine.add_component('RoundedRect', JSON.stringify({
    x: 50,
    y: 50,
    width: 300,
    height: 150,
    radius: 24,
    fill: '#4a90d9',
  }));

  // Initial render
  engine.tick(JSON.stringify(buildQSnapshot()));

  // Event handling with rAF coalescing
  let pendingFrame: number | null = null;

  function scheduleFrame(): void {
    if (pendingFrame === null) {
      pendingFrame = requestAnimationFrame(() => {
        engine.tick(JSON.stringify(buildQSnapshot()));
        pendingFrame = null;
      });
    }
  }

  // Pointer events
  canvas.addEventListener('pointermove', (e) => {
    pointerState.x = e.clientX * dpr;
    pointerState.y = e.clientY * dpr;
    scheduleFrame();
  });

  canvas.addEventListener('pointerdown', () => {
    pointerState.pressed = true;
    scheduleFrame();
  });

  canvas.addEventListener('pointerup', () => {
    pointerState.pressed = false;
    scheduleFrame();
  });

  // Resize handling
  window.addEventListener('resize', () => {
    resizeCanvas();
    engine.resize(canvas.width, canvas.height);
    scheduleFrame();
  });
}

main().catch((error) => {
  console.error('ViewScript initialization failed:', error);
  document.body.innerHTML = `<pre style="color:red;padding:20px;">Error: ${error}</pre>`;
});
