/**
 * ViewScript Counter Application (Bilayer Architecture)
 *
 * Demonstrates the ViewScript architecture:
 * - WebGPU layer: Visual rendering (panel, buttons, glyph meshes)
 * - DOM layer: Interaction + accessibility (transparent text for screen readers)
 *
 * Text rendering uses Q→P glyph path rendering with WASM-side tessellation:
 * 1. Fetch font from Google Fonts (Q-dimension input)
 * 2. Shape and tessellate via WASM FontRegistry (rustybuzz + Loop-Blinn + lyon)
 * 3. Render via gpu-runtime (P-dimension output)
 */

import { initGpu, createRuntime } from '@viewscript/gpu-runtime';
import { getCount, increment, decrement } from './logic.js';
import { parseSvgPath, pathCommandsToJson } from './svg.js';

// =============================================================================
// Font Loading (Google Fonts)
// =============================================================================

// Inter font TTF from Google Fonts (ttf-parser doesn't support WOFF2)
const INTER_TTF_URL = 'https://fonts.gstatic.com/s/inter/v20/UcCO3FwrK3iLTeHuS_nVMrMxCp50SjIw2boKoduKmMEVuLyfMZg.ttf';

async function loadGoogleFont(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to fetch font: ${response.status}`);
  }
  const buffer = await response.arrayBuffer();
  return new Uint8Array(buffer);
}

// =============================================================================
// WebGPU Layer: Mesh Data
// =============================================================================

const QUAD_INDICES = new Uint16Array([0, 1, 2, 2, 3, 0]);

function createQuadVertices(x, y, w, h) {
  return new Float32Array([
    x,     y,     0, 0,
    x + w, y,     1, 0,
    x + w, y + h, 1, 1,
    x,     y + h, 0, 1,
  ]);
}

function registerMeshes(runtime, layout) {
  runtime.registerMesh('panel', {
    pipelineKey: 'solid',
    vertices: createQuadVertices(layout.panel.x, layout.panel.y, layout.panel.w, layout.panel.h),
    indices: QUAD_INDICES,
    color: [0.12, 0.12, 0.18, 1.0],
    vertexCount: 4,
  });

  runtime.registerMesh('incBtn', {
    pipelineKey: 'solid',
    vertices: createQuadVertices(layout.incBtn.x, layout.incBtn.y, layout.incBtn.w, layout.incBtn.h),
    indices: QUAD_INDICES,
    color: [0.65, 0.89, 0.63, 1.0],
    vertexCount: 4,
  });

  runtime.registerMesh('decBtn', {
    pipelineKey: 'solid',
    vertices: createQuadVertices(layout.decBtn.x, layout.decBtn.y, layout.decBtn.w, layout.decBtn.h),
    indices: QUAD_INDICES,
    color: [0.95, 0.55, 0.66, 1.0],
    vertexCount: 4,
  });
}

function updateMeshPositions(runtime, layout) {
  const { panel, incBtn, decBtn } = layout;

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
// Text Mesh Registration (WASM-Tessellated)
// =============================================================================

function registerTextMeshes(runtime, meshIdPrefix, shaped, baseX, baseY, color) {
  const meshIds = [];
  let penX = baseX;
  const yFlip = -1;

  for (let i = 0; i < shaped.glyphs.length; i++) {
    const glyph = shaped.glyphs[i];

    if (glyph.interior.indices.length > 0) {
      const interiorId = `${meshIdPrefix}_${i}_interior`;
      const positions = glyph.interior.positions;
      const vertexCount = positions.length / 2;

      const vertices = new Float32Array(vertexCount * 4);
      for (let v = 0; v < vertexCount; v++) {
        vertices[v * 4 + 0] = penX + positions[v * 2];
        vertices[v * 4 + 1] = baseY + positions[v * 2 + 1] * yFlip;
        vertices[v * 4 + 2] = 0;
        vertices[v * 4 + 3] = 0;
      }

      runtime.registerMesh(interiorId, {
        pipelineKey: 'solid',
        vertices,
        indices: new Uint16Array(glyph.interior.indices),
        color,
        vertexCount,
      });
      meshIds.push(interiorId);
    }

    if (glyph.curves.indices.length > 0) {
      const curvesId = `${meshIdPrefix}_${i}_curves`;
      const positions = glyph.curves.positions;
      const curveSigns = glyph.curves.curve_signs;
      const vertexCount = positions.length / 2;

      if (glyph.curves.curve_uvs) {
        const curveUvs = glyph.curves.curve_uvs;
        const vertices = new Float32Array(vertexCount * 5);
        for (let v = 0; v < vertexCount; v++) {
          vertices[v * 5 + 0] = penX + positions[v * 2];
          vertices[v * 5 + 1] = baseY + positions[v * 2 + 1] * yFlip;
          vertices[v * 5 + 2] = curveUvs[v * 2];
          vertices[v * 5 + 3] = curveUvs[v * 2 + 1];
          vertices[v * 5 + 4] = curveSigns[v];
        }

        runtime.registerMesh(curvesId, {
          pipelineKey: 'loopBlinn',
          vertices,
          indices: new Uint16Array(glyph.curves.indices),
          color,
          vertexCount,
        });
      } else if (glyph.curves.curve_klm) {
        const curveKlm = glyph.curves.curve_klm;
        const vertices = new Float32Array(vertexCount * 6);
        for (let v = 0; v < vertexCount; v++) {
          vertices[v * 6 + 0] = penX + positions[v * 2];
          vertices[v * 6 + 1] = baseY + positions[v * 2 + 1] * yFlip;
          vertices[v * 6 + 2] = curveKlm[v * 3];
          vertices[v * 6 + 3] = curveKlm[v * 3 + 1];
          vertices[v * 6 + 4] = curveKlm[v * 3 + 2];
          vertices[v * 6 + 5] = curveSigns[v];
        }

        runtime.registerMesh(curvesId, {
          pipelineKey: 'loopBlinnCubic',
          vertices,
          indices: new Uint16Array(glyph.curves.indices),
          color,
          vertexCount,
        });
      }
      meshIds.push(curvesId);
    }

    penX += glyph.advance;
  }

  return meshIds;
}

function removeTextMeshes(runtime, meshIds) {
  for (const id of meshIds) {
    runtime.removeMesh(id);
  }
}

// =============================================================================
// WebP Image Loading
// =============================================================================

async function loadWebPImage(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Failed to fetch image: ${response.status}`);
  }
  const blob = await response.blob();
  return createImageBitmap(blob);
}

function registerImageMesh(runtime, meshId, textureId, imageBitmap, x, y, width, height) {
  runtime.registerTexture(textureId, imageBitmap);

  const vertices = new Float32Array([
    x,         y,          0, 0,
    x + width, y,          1, 0,
    x + width, y + height, 1, 1,
    x,         y + height, 0, 1,
  ]);

  runtime.registerTexturedMesh(meshId, {
    pipelineKey: 'texture',
    vertices,
    indices: QUAD_INDICES,
    textureId,
    vertexCount: 4,
  });
}

// =============================================================================
// SVG Path Rendering
// =============================================================================

function registerSvgPathMeshes(runtime, registry, meshIdPrefix, svgPathD, offsetX, offsetY, color, scale = 1.0) {
  const meshIds = [];

  const commandsJson = pathCommandsToJson(parseSvgPath(svgPathD));
  const resultJson = registry.tessellate_svg_path(commandsJson);
  const tessellated = JSON.parse(resultJson);

  if (tessellated.interior.indices.length > 0) {
    const interiorId = `${meshIdPrefix}_interior`;
    const positions = tessellated.interior.positions;
    const vertexCount = positions.length / 2;

    const vertices = new Float32Array(vertexCount * 4);
    for (let v = 0; v < vertexCount; v++) {
      vertices[v * 4 + 0] = offsetX + positions[v * 2] * scale;
      vertices[v * 4 + 1] = offsetY + positions[v * 2 + 1] * scale;
      vertices[v * 4 + 2] = 0;
      vertices[v * 4 + 3] = 0;
    }

    runtime.registerMesh(interiorId, {
      pipelineKey: 'solid',
      vertices,
      indices: new Uint16Array(tessellated.interior.indices),
      color,
      vertexCount,
    });
    meshIds.push(interiorId);
  }

  if (tessellated.curves.indices.length > 0) {
    const curvesId = `${meshIdPrefix}_curves`;
    const positions = tessellated.curves.positions;
    const curveSigns = tessellated.curves.curve_signs;
    const vertexCount = positions.length / 2;

    if (tessellated.curves.curve_uvs) {
      const curveUvs = tessellated.curves.curve_uvs;
      const vertices = new Float32Array(vertexCount * 5);
      for (let v = 0; v < vertexCount; v++) {
        vertices[v * 5 + 0] = offsetX + positions[v * 2] * scale;
        vertices[v * 5 + 1] = offsetY + positions[v * 2 + 1] * scale;
        vertices[v * 5 + 2] = curveUvs[v * 2];
        vertices[v * 5 + 3] = curveUvs[v * 2 + 1];
        vertices[v * 5 + 4] = curveSigns[v];
      }

      runtime.registerMesh(curvesId, {
        pipelineKey: 'loopBlinn',
        vertices,
        indices: new Uint16Array(tessellated.curves.indices),
        color,
        vertexCount,
      });
    } else if (tessellated.curves.curve_klm) {
      const curveKlm = tessellated.curves.curve_klm;
      const vertices = new Float32Array(vertexCount * 6);
      for (let v = 0; v < vertexCount; v++) {
        vertices[v * 6 + 0] = offsetX + positions[v * 2] * scale;
        vertices[v * 6 + 1] = offsetY + positions[v * 2 + 1] * scale;
        vertices[v * 6 + 2] = curveKlm[v * 3];
        vertices[v * 6 + 3] = curveKlm[v * 3 + 1];
        vertices[v * 6 + 4] = curveKlm[v * 3 + 2];
        vertices[v * 6 + 5] = curveSigns[v];
      }

      runtime.registerMesh(curvesId, {
        pipelineKey: 'loopBlinnCubic',
        vertices,
        indices: new Uint16Array(tessellated.curves.indices),
        color,
        vertexCount,
      });
    }
    meshIds.push(curvesId);
  }

  return meshIds;
}

// =============================================================================
// Demo Assets
// =============================================================================

const DEMO_IMAGE_URL = '/demo.webp';
const TIGER_SVG_URL = '/tiger.svg';

// =============================================================================
// SVG File Parser
// =============================================================================

function parseColor(color) {
  if (color === 'none' || !color) return [0, 0, 0, 0];

  if (color.startsWith('#')) {
    const hex = color.slice(1);
    if (hex.length === 3) {
      const r = parseInt(hex[0] + hex[0], 16) / 255;
      const g = parseInt(hex[1] + hex[1], 16) / 255;
      const b = parseInt(hex[2] + hex[2], 16) / 255;
      return [r, g, b, 1];
    } else if (hex.length === 6) {
      const r = parseInt(hex.slice(0, 2), 16) / 255;
      const g = parseInt(hex.slice(2, 4), 16) / 255;
      const b = parseInt(hex.slice(4, 6), 16) / 255;
      return [r, g, b, 1];
    }
  }

  return [0.5, 0.5, 0.5, 1];
}

async function loadSvgPaths(url) {
  const response = await fetch(url);
  const svgText = await response.text();

  const parser = new DOMParser();
  const doc = parser.parseFromString(svgText, 'image/svg+xml');

  const paths = [];
  const pathElements = doc.querySelectorAll('path');

  for (const pathEl of pathElements) {
    const d = pathEl.getAttribute('d');
    if (!d) continue;

    let fill = pathEl.getAttribute('fill');
    if (!fill) {
      const parent = pathEl.closest('g[fill]');
      fill = parent?.getAttribute('fill') || '#888';
    }

    if (fill === 'none') continue;

    paths.push({
      d,
      fill: parseColor(fill),
    });
  }

  return paths;
}

// =============================================================================
// DOM Layer Setup
// =============================================================================

function mountDOM(container) {
  const overlay = document.createElement('div');
  overlay.style.cssText = 'position:absolute;inset:0;pointer-events:none;overflow:hidden;';
  overlay.setAttribute('aria-live', 'polite');

  const incBtn = document.createElement('button');
  incBtn.style.cssText = 'position:absolute;background:transparent;border:none;cursor:pointer;pointer-events:auto;';
  incBtn.setAttribute('aria-label', 'Increment counter');
  overlay.appendChild(incBtn);

  const decBtn = document.createElement('button');
  decBtn.style.cssText = 'position:absolute;background:transparent;border:none;cursor:pointer;pointer-events:auto;';
  decBtn.setAttribute('aria-label', 'Decrement counter');
  overlay.appendChild(decBtn);

  const incLabel = document.createElement('span');
  incLabel.style.cssText = 'position:absolute;pointer-events:none;color:transparent;font-family:Inter,system-ui,sans-serif;font-size:32px;font-weight:bold;';
  incLabel.textContent = '+';
  overlay.appendChild(incLabel);

  const decLabel = document.createElement('span');
  decLabel.style.cssText = 'position:absolute;pointer-events:none;color:transparent;font-family:Inter,system-ui,sans-serif;font-size:32px;font-weight:bold;';
  decLabel.textContent = '-';
  overlay.appendChild(decLabel);

  const countLabel = document.createElement('span');
  countLabel.style.cssText = 'position:absolute;pointer-events:none;color:transparent;font-family:Inter,system-ui,sans-serif;font-size:72px;font-weight:bold;text-align:center;';
  countLabel.setAttribute('aria-label', 'Counter value');
  countLabel.textContent = String(getCount());
  overlay.appendChild(countLabel);

  container.appendChild(overlay);
  return { overlay, incBtn, decBtn, incLabel, decLabel, countLabel };
}

function updateDOM(dom, layout) {
  dom.incBtn.style.transform = `translate3d(${layout.incBtn.x}px, ${layout.incBtn.y}px, 0)`;
  dom.incBtn.style.width = `${layout.incBtn.w}px`;
  dom.incBtn.style.height = `${layout.incBtn.h}px`;

  dom.decBtn.style.transform = `translate3d(${layout.decBtn.x}px, ${layout.decBtn.y}px, 0)`;
  dom.decBtn.style.width = `${layout.decBtn.w}px`;
  dom.decBtn.style.height = `${layout.decBtn.h}px`;

  dom.incLabel.style.transform = `translate3d(${layout.incBtn.x + layout.incBtn.w / 2 - 10}px, ${layout.incBtn.y + 8}px, 0)`;
  dom.decLabel.style.transform = `translate3d(${layout.decBtn.x + layout.decBtn.w / 2 - 8}px, ${layout.decBtn.y + 8}px, 0)`;

  dom.countLabel.style.transform = `translate3d(${layout.label.x}px, ${layout.label.y}px, 0)`;
  dom.countLabel.style.width = `${layout.label.w}px`;
}

// =============================================================================
// Layout Metrics
// =============================================================================

function computeLayout(viewportWidth, viewportHeight) {
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
    incBtn: { x: panelX + panelWidth - btnSpacing - btnWidth, y: btnY, w: btnWidth, h: btnHeight },
    decBtn: { x: panelX + btnSpacing, y: btnY, w: btnWidth, h: btnHeight },
    label: { x: panelX + panelWidth / 2 - 50, y: panelY + 40, w: 100, h: 80 },
  };
}

// =============================================================================
// WASM FontRegistry Interface
// =============================================================================

let fontRegistry = null;

async function initFontRegistry() {
  if (fontRegistry) return fontRegistry;

  const wasm = await import('@viewscript/wasm');
  await wasm.default();

  fontRegistry = new wasm.FontRegistry();
  return fontRegistry;
}

async function loadAndRegisterFont(registry) {
  console.log('[ViewScript] Loading Inter font from Google Fonts...');
  const fontData = await loadGoogleFont(INTER_TTF_URL);
  console.log(`[ViewScript] Font loaded: ${fontData.length} bytes`);
  registry.register('Inter', fontData);
}

function shapeAndTessellate(registry, text, fontSize) {
  const json = registry.shape_and_tessellate('Inter', text, fontSize);
  return JSON.parse(json);
}

// =============================================================================
// Main Entry Point
// =============================================================================

async function mount(container) {
  const canvas = document.createElement('canvas');
  canvas.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;';
  container.style.position = 'relative';
  container.appendChild(canvas);

  const resizeCanvas = () => {
    const rect = container.getBoundingClientRect();
    canvas.width = rect.width * devicePixelRatio;
    canvas.height = rect.height * devicePixelRatio;
  };
  resizeCanvas();
  window.addEventListener('resize', resizeCanvas);

  const [gpu, registry] = await Promise.all([
    initGpu(canvas),
    initFontRegistry(),
  ]);

  const runtime = createRuntime(gpu);

  await loadAndRegisterFont(registry);

  let layout = computeLayout(container.clientWidth, container.clientHeight);

  registerMeshes(runtime, layout);

  const dom = mountDOM(container);
  updateDOM(dom, layout);

  let counterMeshIds = [];
  let incBtnMeshIds = [];
  let decBtnMeshIds = [];

  const COUNTER_FONT_SIZE = 72;
  const BTN_FONT_SIZE = 32;
  const TEXT_COLOR = [0.80, 0.84, 0.96, 1.0];
  const BTN_TEXT_COLOR = [0.12, 0.12, 0.18, 1.0];

  const updateCounterMesh = () => {
    removeTextMeshes(runtime, counterMeshIds);
    const text = String(getCount());
    const shaped = shapeAndTessellate(registry, text, COUNTER_FONT_SIZE);
    const labelCenterX = layout.label.x + layout.label.w / 2;
    const textStartX = labelCenterX - shaped.total_advance / 2;
    const textY = layout.label.y + COUNTER_FONT_SIZE;
    counterMeshIds = registerTextMeshes(runtime, 'counter', shaped, textStartX, textY, TEXT_COLOR);
  };

  const updateButtonMeshes = () => {
    removeTextMeshes(runtime, incBtnMeshIds);
    removeTextMeshes(runtime, decBtnMeshIds);

    const incShaped = shapeAndTessellate(registry, '+', BTN_FONT_SIZE);
    const decShaped = shapeAndTessellate(registry, '-', BTN_FONT_SIZE);

    const incCenterX = layout.incBtn.x + layout.incBtn.w / 2;
    const incTextX = incCenterX - incShaped.total_advance / 2;
    const incTextY = layout.incBtn.y + layout.incBtn.h / 2 + BTN_FONT_SIZE / 3;

    const decCenterX = layout.decBtn.x + layout.decBtn.w / 2;
    const decTextX = decCenterX - decShaped.total_advance / 2;
    const decTextY = layout.decBtn.y + layout.decBtn.h / 2 + BTN_FONT_SIZE / 3;

    incBtnMeshIds = registerTextMeshes(runtime, 'incBtn_text', incShaped, incTextX, incTextY, BTN_TEXT_COLOR);
    decBtnMeshIds = registerTextMeshes(runtime, 'decBtn_text', decShaped, decTextX, decTextY, BTN_TEXT_COLOR);
  };

  updateCounterMesh();
  updateButtonMeshes();

  // =============================================================================
  // Demo: WebP Image (texture pipeline)
  // =============================================================================

  let imgState = {
    loaded: false,
    bitmap: null,
    x: 20,
    y: container.clientHeight - 150 - 20,
    width: 200,
    height: 150,
    dragging: false,
    dragOffsetX: 0,
    dragOffsetY: 0,
  };

  const updateImageMesh = () => {
    if (!imgState.loaded || !imgState.bitmap) return;
    runtime.removeTexturedMesh('demo_webp');
    runtime.removeTexture('webp_texture');
    registerImageMesh(runtime, 'demo_webp', 'webp_texture', imgState.bitmap, imgState.x, imgState.y, imgState.width, imgState.height);
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
  };

  loadWebPImage(DEMO_IMAGE_URL)
    .then((imageBitmap) => {
      imgState.bitmap = imageBitmap;
      imgState.loaded = true;
      registerImageMesh(runtime, 'demo_webp', 'webp_texture', imageBitmap, imgState.x, imgState.y, imgState.width, imgState.height);
      console.log('[ViewScript] WebP image loaded and registered');
      runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
    })
    .catch((err) => {
      console.warn('[ViewScript] Failed to load WebP demo image:', err);
    });

  canvas.addEventListener('mousedown', (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    if (imgState.loaded && mx >= imgState.x && mx <= imgState.x + imgState.width && my >= imgState.y && my <= imgState.y + imgState.height) {
      imgState.dragging = true;
      imgState.dragOffsetX = mx - imgState.x;
      imgState.dragOffsetY = my - imgState.y;
      canvas.style.cursor = 'grabbing';
    }
  });

  canvas.addEventListener('mousemove', (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    if (imgState.dragging) {
      imgState.x = mx - imgState.dragOffsetX;
      imgState.y = my - imgState.dragOffsetY;
      updateImageMesh();
    } else if (imgState.loaded && mx >= imgState.x && mx <= imgState.x + imgState.width && my >= imgState.y && my <= imgState.y + imgState.height) {
      canvas.style.cursor = 'grab';
    } else {
      canvas.style.cursor = 'default';
    }
  });

  canvas.addEventListener('mouseup', () => {
    if (imgState.dragging) {
      imgState.dragging = false;
      canvas.style.cursor = 'grab';
    }
  });

  canvas.addEventListener('mouseleave', () => {
    if (imgState.dragging) {
      imgState.dragging = false;
      canvas.style.cursor = 'default';
    }
  });

  // =============================================================================
  // Demo: SVG Path (Loop-Blinn + lyon tessellation) - Ghostscript Tiger
  // =============================================================================

  const svgState = {
    loaded: false,
    paths: [],
    meshIds: [],
    x: container.clientWidth - 350,
    y: container.clientHeight - 400,
    width: 300,
    height: 300,
    scale: 0.35,
    dragging: false,
    dragOffsetX: 0,
    dragOffsetY: 0,
  };

  const removeSvgMeshes = () => {
    for (const id of svgState.meshIds) {
      runtime.removeMesh(id);
    }
    svgState.meshIds = [];
  };

  const renderSvgMeshes = () => {
    if (!svgState.loaded) return;
    removeSvgMeshes();

    let pathIndex = 0;
    let successCount = 0;
    let failCount = 0;

    for (const pathData of svgState.paths) {
      try {
        const meshIds = registerSvgPathMeshes(runtime, registry, `tiger_${pathIndex}`, pathData.d, svgState.x, svgState.y, pathData.fill, svgState.scale);
        svgState.meshIds.push(...meshIds);
        successCount++;
      } catch (err) {
        failCount++;
      }
      pathIndex++;
    }

    console.log(`[ViewScript] Tiger: ${successCount} paths rendered, ${failCount} failed`);
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
  };

  loadSvgPaths(TIGER_SVG_URL)
    .then((paths) => {
      svgState.paths = paths;
      svgState.loaded = true;
      console.log(`[ViewScript] Tiger SVG loaded: ${paths.length} paths`);
      renderSvgMeshes();
    })
    .catch((err) => {
      console.warn('[ViewScript] Failed to load Tiger SVG:', err);
    });

  canvas.addEventListener('mousedown', (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    if (svgState.loaded && mx >= svgState.x && mx <= svgState.x + svgState.width && my >= svgState.y && my <= svgState.y + svgState.height) {
      svgState.dragging = true;
      svgState.dragOffsetX = mx - svgState.x;
      svgState.dragOffsetY = my - svgState.y;
      canvas.style.cursor = 'grabbing';
      e.stopPropagation();
    }
  });

  canvas.addEventListener('mousemove', (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    if (svgState.dragging) {
      svgState.x = mx - svgState.dragOffsetX;
      svgState.y = my - svgState.dragOffsetY;
      renderSvgMeshes();
    } else if (svgState.loaded && !imgState.dragging && mx >= svgState.x && mx <= svgState.x + svgState.width && my >= svgState.y && my <= svgState.y + svgState.height) {
      canvas.style.cursor = 'grab';
    }
  });

  canvas.addEventListener('mouseup', () => {
    if (svgState.dragging) {
      svgState.dragging = false;
      canvas.style.cursor = 'grab';
    }
  });

  canvas.addEventListener('mouseleave', () => {
    if (svgState.dragging) {
      svgState.dragging = false;
    }
  });

  dom.incBtn.addEventListener('click', () => {
    increment();
    dom.countLabel.textContent = String(getCount());
    updateCounterMesh();
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
  });

  dom.decBtn.addEventListener('click', () => {
    decrement();
    dom.countLabel.textContent = String(getCount());
    updateCounterMesh();
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
  });

  window.addEventListener('resize', () => {
    layout = computeLayout(container.clientWidth, container.clientHeight);
    updateDOM(dom, layout);
    updateMeshPositions(runtime, layout);
    updateCounterMesh();
    updateButtonMeshes();
    runtime.render({ r: 0.067, g: 0.067, b: 0.106, a: 1 });
  });

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
  .then(() => {
    console.log('ViewScript mounted (bilayer architecture)');
    console.log('- WebGPU layer: gpu-runtime (panel, buttons, glyph meshes)');
    console.log('- DOM layer: transparent text (accessibility)');
    console.log('- Text rendering: WASM-side Loop-Blinn + lyon tessellation');
    console.log('- WebP images: texture pipeline');
    console.log('- SVG paths: JS parser + WASM tessellation');
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
