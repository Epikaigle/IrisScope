import * as THREE from "./vendor/three.module.js";

// Dimensions follow the published 17 × 3.6 × 4 cm proportions. The shell,
// opaque eyecup and physical controls are reconstructed from Firefly photos.
// This is an illustrative model, not a manufacturer CAD file.
function createDE400() {
  const device = new THREE.Group();
  const shell = new THREE.MeshStandardMaterial({
    color: 0x171b1c,
    roughness: 0.53,
    metalness: 0.12,
  });
  const rubber = new THREE.MeshStandardMaterial({
    color: 0x111414,
    roughness: 0.86,
  });
  const inset = new THREE.MeshStandardMaterial({
    color: 0x060908,
    roughness: 0.78,
  });
  const silver = new THREE.MeshStandardMaterial({
    color: 0xc9cccb,
    roughness: 0.4,
    metalness: 0.55,
  });
  const add = (geometry, material, x = 0, y = 0, z = 0) => {
    const mesh = new THREE.Mesh(geometry, material);
    mesh.position.set(x, y, z);
    device.add(mesh);
    return mesh;
  };
  const profile = [
    [0, -3.45],
    [0.57, -3.45],
    [0.61, -3.25],
    [0.66, -2.8],
    [0.77, -2],
    [0.86, -1.25],
    [0.91, -0.25],
    [0.92, 0.6],
    [0.89, 1.3],
    [0.83, 2.1],
    [0.72, 3.05],
    [0.62, 3.65],
    [0.46, 3.92],
    [0.23, 4.08],
    [0, 4.12],
  ];
  const body = add(
    new THREE.LatheGeometry(
      profile.map(([r, x]) => new THREE.Vector2(r, x)),
      96,
    ),
    shell,
  );
  body.rotation.z = -Math.PI / 2;
  body.scale.x = 0.96;
  for (const [x, r] of [
    [-1.3, 0.86],
    [1.3, 0.886],
    [2.78, 0.749],
  ]) {
    const seam = add(new THREE.TorusGeometry(r, 0.012, 8, 96), inset, x);
    seam.rotation.y = Math.PI / 2;
    seam.scale.x = 0.96;
  }

  // Hollow, flared rubber cup with a saddle-shaped oval lip. Both surfaces
  // and the rounded rim are geometry, so the inside remains visible on rotation.
  const segments = 96;
  const rings = [
    [-3.35, 0.6, 0.59],
    [-3.7, 0.7, 0.65],
    [-4.12, 0.91, 0.75],
    [-4.48, 1.19, 0.86],
    [-4.51, 1.2, 0.87],
    [-4.48, 1.08, 0.76],
    [-4.1, 0.79, 0.65],
    [-3.69, 0.6, 0.55],
    [-3.35, 0.49, 0.49],
  ];
  const vertices = [],
    indices = [];
  for (let ring = 0; ring < rings.length; ring++) {
    const [x, ry, rz] = rings[ring];
    const saddle = ring >= 2 && ring <= 6 ? 0.19 : 0;
    for (let i = 0; i <= segments; i++) {
      const theta = (i / segments) * Math.PI * 2;
      vertices.push(
        x + saddle * Math.cos(theta * 2),
        Math.cos(theta) * ry,
        Math.sin(theta) * rz,
      );
    }
  }
  for (let ring = 0; ring < rings.length - 1; ring++) {
    for (let i = 0; i < segments; i++) {
      const a = ring * (segments + 1) + i,
        b = a + segments + 1;
      indices.push(a, b, a + 1, b, b + 1, a + 1);
    }
  }
  const cupGeometry = new THREE.BufferGeometry();
  cupGeometry.setAttribute(
    "position",
    new THREE.Float32BufferAttribute(vertices, 3),
  );
  cupGeometry.setIndex(indices);
  cupGeometry.computeVertexNormals();
  add(
    cupGeometry,
    new THREE.MeshStandardMaterial({
      color: 0x131817,
      roughness: 0.82,
      side: THREE.DoubleSide,
    }),
  );
  const rimPoints = Array.from({ length: 96 }, (_, i) => {
    const t = (i / 96) * Math.PI * 2;
    return new THREE.Vector3(
      -4.5 + 0.19 * Math.cos(2 * t),
      Math.cos(t) * 1.15,
      Math.sin(t) * 0.82,
    );
  });
  add(
    new THREE.TubeGeometry(
      new THREE.CatmullRomCurve3(rimPoints, true),
      96,
      0.062,
      8,
      true,
    ),
    rubber,
  );

  function frontDisk(radius, material, x, y = 0, z = 0) {
    const disk = add(
      new THREE.CylinderGeometry(radius, radius, 0.06, 48),
      material,
      x,
      y,
      z,
    );
    disk.rotation.z = Math.PI / 2;
    return disk;
  }
  frontDisk(0.49, inset, -3.77);
  frontDisk(0.32, silver, -3.81);
  frontDisk(
    0.28,
    new THREE.MeshPhysicalMaterial({
      color: 0x123342,
      roughness: 0.14,
      metalness: 0.18,
      clearcoat: 1,
    }),
    -3.86,
  );
  frontDisk(
    0.18,
    new THREE.MeshStandardMaterial({
      color: 0x061a24,
      roughness: 0.12,
      metalness: 0.3,
    }),
    -3.9,
  );
  const ledMaterial = new THREE.MeshStandardMaterial({
    color: 0xffffed,
    emissive: 0xfff4d8,
    emissiveIntensity: 1.8,
  });
  for (let i = 0; i < 4; i++) {
    const angle = Math.PI / 4 + (i * Math.PI) / 2;
    frontDisk(
      0.063,
      ledMaterial,
      -3.84,
      Math.cos(angle) * 0.4,
      Math.sin(angle) * 0.4,
    );
  }

  // The exposed silver focus wheel sits in a black oval recess, rather than
  // forming a chrome ring around the body.
  const recess = add(
    new THREE.SphereGeometry(1, 40, 20),
    inset,
    -0.12,
    0.872,
    0,
  );
  recess.scale.set(0.93, 0.105, 0.38);
  const wheel = add(
    new THREE.SphereGeometry(1, 40, 20),
    silver,
    -0.12,
    0.953,
    0,
  );
  wheel.scale.set(0.73, 0.095, 0.28);
  for (let i = -3; i <= 3; i++) {
    const z = i * 0.072;
    const halfLength = 0.66 * Math.sqrt(1 - (z / 0.3) ** 2);
    const rib = add(
      new THREE.CylinderGeometry(0.013, 0.013, 2 * halfLength, 8),
      shell,
      -0.12,
      1.045 - Math.abs(z) * 0.2,
      z,
    );
    rib.rotation.z = Math.PI / 2;
  }

  function labelTexture(width, height, draw) {
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    draw(canvas.getContext("2d"), width, height);
    const texture = new THREE.CanvasTexture(canvas);
    texture.colorSpace = THREE.SRGBColorSpace;
    texture.anisotropy = 4;
    return texture;
  }
  const logoTexture = labelTexture(512, 160, (context) => {
    context.fillStyle = "#c9ccca";
    context.font = "100px Georgia";
    context.textAlign = "center";
    context.textBaseline = "middle";
    context.fillText("Firefly", 256, 78);
  });
  add(
    new THREE.PlaneGeometry(1.5, 0.47),
    new THREE.MeshBasicMaterial({
      map: logoTexture,
      transparent: true,
      depthWrite: false,
    }),
    0.02,
    -0.1,
    0.908,
  );

  // Rear top Snapshot button, with the camera symbol shown on the real unit.
  const button = add(
    new THREE.CylinderGeometry(0.25, 0.25, 0.035, 48),
    inset,
    2.75,
    0.75,
    0,
  );
  button.rotation.z = -0.11;
  const cameraTexture = labelTexture(128, 128, (context) => {
    context.strokeStyle = "#b7bfbb";
    context.lineWidth = 7;
    context.beginPath();
    context.roundRect(29, 44, 70, 47, 8);
    context.stroke();
    context.beginPath();
    context.arc(64, 67, 14, 0, Math.PI * 2);
    context.stroke();
    context.beginPath();
    context.moveTo(46, 44);
    context.lineTo(50, 32);
    context.lineTo(76, 32);
    context.lineTo(82, 44);
    context.stroke();
  });
  const icon = add(
    new THREE.PlaneGeometry(0.37, 0.37),
    new THREE.MeshBasicMaterial({
      map: cameraTexture,
      transparent: true,
      depthWrite: false,
    }),
    2.75,
    0.774,
    0,
  );
  icon.rotation.x = -Math.PI / 2;
  icon.rotation.y = -0.11;

  // Side brightness thumbwheel and its short white brightness scale.
  const brightnessInset = add(
    new THREE.SphereGeometry(1, 32, 16),
    inset,
    2.7,
    -0.02,
    0.732,
  );
  brightnessInset.scale.set(0.4, 0.12, 0.08);
  for (let i = -4; i <= 4; i++) {
    const grip = add(
      new THREE.BoxGeometry(0.035, 0.14, 0.028),
      silver,
      2.7 + i * 0.063,
      -0.015,
      0.8,
    );
    grip.material = new THREE.MeshStandardMaterial({
      color: 0x777e78,
      roughness: 0.7,
    });
  }
  const scaleTexture = labelTexture(256, 96, (context) => {
    context.fillStyle = "#c4ccc5";
    context.font = "34px sans-serif";
    context.fillText("☀", 8, 64);
    for (let i = 0; i < 5; i++)
      context.fillRect(73 + i * 26, 65 - i * 9, 7, 13 + i * 9);
  });
  add(
    new THREE.PlaneGeometry(0.76, 0.29),
    new THREE.MeshBasicMaterial({
      map: scaleTexture,
      transparent: true,
      depthWrite: false,
    }),
    2.7,
    -0.25,
    0.745,
  );

  const relief = add(
    new THREE.CylinderGeometry(0.09, 0.12, 0.38, 16),
    rubber,
    4.18,
  );
  relief.rotation.z = Math.PI / 2;
  for (let i = 0; i < 4; i++) {
    const ring = add(
      new THREE.TorusGeometry(0.11 - i * 0.005, 0.012, 6, 16),
      rubber,
      4.08 + i * 0.08,
    );
    ring.rotation.y = Math.PI / 2;
  }
  const cableCurve = new THREE.CatmullRomCurve3([
    new THREE.Vector3(4.3, 0, 0),
    new THREE.Vector3(4.9, -0.05, 0),
    new THREE.Vector3(5.25, -0.7, -0.08),
    new THREE.Vector3(5.0, -1.35, -0.2),
    new THREE.Vector3(4.2, -1.8, -0.3),
    new THREE.Vector3(3.2, -1.9, -0.45),
  ]);
  add(new THREE.TubeGeometry(cableCurve, 64, 0.063, 10, false), rubber);
  return { device, ledMaterial };
}

export function initDeviceViewer() {
  const stage = document.getElementById("model-stage");
  const canvas = document.getElementById("device-canvas");
  const fallback = document.getElementById("model-fallback");
  const controls = document.getElementById("model-controls");
  const tag = document.getElementById("view-tag");
  const hint = document.getElementById("model-hint");
  let renderer,
    contextAvailable = false;
  function showFallback() {
    contextAvailable = false;
    canvas.hidden = true;
    fallback.hidden = false;
    controls.hidden = true;
    tag.textContent = "Photographie";
    hint.textContent = "L’appareil réel, photographié par Firefly Global.";
    cancelAnimationFrame(frameId);
    frameId = 0;
  }
  let frameId = 0;
  try {
    renderer = new THREE.WebGLRenderer({
      canvas,
      alpha: true,
      antialias: true,
      powerPreference: "low-power",
    });
    contextAvailable = true;
  } catch {
    showFallback();
    return;
  }
  let pixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
  renderer.setPixelRatio(pixelRatio);
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.25;
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(27, 1, 0.1, 100);
  const { device, ledMaterial } = createDE400();
  scene.add(device);
  scene.add(new THREE.HemisphereLight(0xf6f3e5, 0x657669, 2.4));
  const key = new THREE.DirectionalLight(0xfff4db, 4.0);
  key.position.set(-4, 7, 6);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xb6c9ce, 3.0);
  fill.position.set(2, 3, -6);
  scene.add(fill);
  const edge = new THREE.DirectionalLight(0xffffff, 1.8);
  edge.position.set(5, -2, 4);
  scene.add(edge);
  const initial = { yaw: 0.65, pitch: -0.05, distance: 17.5 };
  let yaw = initial.yaw,
    pitch = initial.pitch,
    distance = initial.distance;
  let autoRotate = false,
    visible = true,
    disposed = false,
    dirty = true,
    lastFrame = 0,
    renderedWidth = 0,
    renderedHeight = 0;
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const rotateButton = document.getElementById("model-rotate");
  const lightButton = document.getElementById("model-light");
  const zoomIn = document.getElementById("model-zoom-in");
  const zoomOut = document.getElementById("model-zoom-out");
  let gesture = null;
  function requestRender() {
    dirty = true;
    if (
      !frameId &&
      !disposed &&
      contextAvailable &&
      visible &&
      !document.hidden
    )
      frameId = requestAnimationFrame(render);
  }
  function syncSize() {
    const bounds = stage.getBoundingClientRect();
    const width = Math.round(bounds.width),
      height = Math.round(bounds.height);
    if (!width || !height) return false;
    const nextPixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
    // Assigning canvas.width/height clears its image, even if a resize observer
    // reports the same dimensions. Resize only when the drawing buffer changes.
    if (
      width !== renderedWidth ||
      height !== renderedHeight ||
      nextPixelRatio !== pixelRatio
    ) {
      if (nextPixelRatio !== pixelRatio) {
        pixelRatio = nextPixelRatio;
        renderer.setPixelRatio(pixelRatio);
      }
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderedWidth = width;
      renderedHeight = height;
      dirty = true;
    }
    return true;
  }
  function render(now) {
    frameId = 0;
    if (disposed || !contextAvailable || !visible || document.hidden) return;
    if (now - lastFrame < 1000 / 60) {
      frameId = requestAnimationFrame(render);
      return;
    }
    if (!syncSize()) return;
    const elapsed = lastFrame ? Math.min((now - lastFrame) / 1000, 0.05) : 0;
    lastFrame = now;
    if (autoRotate && !gesture) {
      yaw += elapsed * 0.18;
      dirty = true;
    }
    if (dirty) {
      device.rotation.set(pitch, yaw, -0.1);
      const framedDistance = distance * Math.max(1, 1.55 / camera.aspect);
      camera.position.set(
        -framedDistance * 0.24,
        framedDistance * 0.26,
        framedDistance * 0.92,
      );
      camera.lookAt(0, -0.15, 0);
      renderer.render(scene, camera);
      dirty = false;
    }
    if (autoRotate) frameId = requestAnimationFrame(render);
  }
  function stopRotation() {
    autoRotate = false;
    rotateButton.setAttribute("aria-pressed", "false");
  }
  function zoom(amount) {
    distance = THREE.MathUtils.clamp(distance + amount, 15.5, 26);
    zoomIn.disabled = distance <= 15.5;
    zoomOut.disabled = distance >= 26;
    requestRender();
  }
  function reset() {
    stopRotation();
    gesture = null;
    yaw = initial.yaw;
    pitch = initial.pitch;
    distance = initial.distance;
    zoom(0);
  }
  rotateButton.addEventListener("click", () => {
    autoRotate = !autoRotate;
    rotateButton.setAttribute("aria-pressed", String(autoRotate));
    requestRender();
  });
  lightButton.addEventListener("click", () => {
    const on = lightButton.getAttribute("aria-pressed") !== "true";
    lightButton.setAttribute("aria-pressed", String(on));
    lightButton.textContent = on ? "LED allumées" : "LED éteintes";
    ledMaterial.emissiveIntensity = on ? 1.8 : 0;
    ledMaterial.color.setHex(on ? 0xffffed : 0x8d9287);
    requestRender();
  });
  document.getElementById("model-reset").addEventListener("click", reset);
  zoomIn.addEventListener("click", () => zoom(-1.5));
  zoomOut.addEventListener("click", () => zoom(1.5));
  canvas.addEventListener("pointerdown", (event) => {
    if (
      !event.isPrimary ||
      (event.pointerType === "mouse" && event.button !== 0)
    )
      return;
    stopRotation();
    gesture = {
      id: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      yaw,
      pitch,
      active: event.pointerType === "mouse",
    };
    if (gesture.active) canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener("pointermove", (event) => {
    if (!gesture || event.pointerId !== gesture.id) return;
    const dx = event.clientX - gesture.x,
      dy = event.clientY - gesture.y;
    // Let vertical touches scroll the page; only horizontal touches rotate.
    if (!gesture.active) {
      if (Math.abs(dy) > Math.abs(dx) && Math.abs(dy) > 8) {
        gesture = null;
        return;
      }
      if (Math.abs(dx) < 8) return;
      gesture.active = true;
      canvas.setPointerCapture(event.pointerId);
    }
    yaw = gesture.yaw + dx * 0.009;
    if (event.pointerType === "mouse")
      pitch = THREE.MathUtils.clamp(gesture.pitch + dy * 0.006, -1.15, 0.75);
    requestRender();
  });
  const endGesture = () => {
    gesture = null;
  };
  canvas.addEventListener("pointerup", endGesture);
  canvas.addEventListener("pointercancel", endGesture);
  canvas.addEventListener("lostpointercapture", endGesture);
  canvas.addEventListener("keydown", (event) => {
    // Keep browser/page shortcuts, such as Ctrl+Home, available while the
    // viewer has focus. Plain Home still resets the model as documented.
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    let handled = true;
    if (event.key === "ArrowLeft") yaw -= 0.18;
    else if (event.key === "ArrowRight") yaw += 0.18;
    else if (event.key === "ArrowUp") pitch = Math.max(-1.15, pitch - 0.12);
    else if (event.key === "ArrowDown") pitch = Math.min(0.75, pitch + 0.12);
    else if (event.key === "+" || event.key === "=") zoom(-1.5);
    else if (event.key === "-") zoom(1.5);
    else if (event.key === "0" || event.key === "Home") reset();
    else handled = false;
    if (handled) {
      event.preventDefault();
      stopRotation();
      requestRender();
    }
  });
  const resizeObserver = new ResizeObserver(requestRender);
  resizeObserver.observe(stage);
  // A window/DPR or mobile visual-viewport change can happen without changing
  // the stage's CSS box. Redraw the visible, static scene when that happens.
  window.addEventListener("resize", requestRender);
  window.visualViewport?.addEventListener("resize", requestRender);
  window.addEventListener("pageshow", () => {
    lastFrame = 0;
    requestRender();
  });
  const intersectionObserver = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    if (visible) {
      lastFrame = 0;
      requestRender();
    } else {
      cancelAnimationFrame(frameId);
      frameId = 0;
    }
  });
  intersectionObserver.observe(stage);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
      gesture = null;
      cancelAnimationFrame(frameId);
      frameId = 0;
    } else {
      lastFrame = 0;
      requestRender();
    }
  });
  reducedMotion.addEventListener("change", () => {
    if (reducedMotion.matches) {
      stopRotation();
      requestRender();
    }
  });
  canvas.addEventListener("webglcontextlost", (event) => {
    event.preventDefault();
    showFallback();
  });
  canvas.addEventListener("webglcontextrestored", () => {
    contextAvailable = true;
    // The restored renderer needs a fresh buffer and frame, even at the same
    // CSS dimensions as before the context was lost.
    renderedWidth = 0;
    renderedHeight = 0;
    lastFrame = 0;
    fallback.hidden = true;
    canvas.hidden = false;
    controls.hidden = false;
    tag.textContent = "Vue 3D";
    hint.textContent =
      "Faites glisser horizontalement pour tourner. Les boutons permettent de zoomer.";
    requestRender();
  });
  window.addEventListener("pagehide", (event) => {
    if (event.persisted) return;
    disposed = true;
    cancelAnimationFrame(frameId);
    window.removeEventListener("resize", requestRender);
    window.visualViewport?.removeEventListener("resize", requestRender);
    resizeObserver.disconnect();
    intersectionObserver.disconnect();
    const geometries = new Set(),
      materials = new Set(),
      textures = new Set();
    scene.traverse((object) => {
      if (!object.isMesh) return;
      geometries.add(object.geometry);
      for (const material of [].concat(object.material)) {
        materials.add(material);
        if (material.map) textures.add(material.map);
      }
    });
    geometries.forEach((geometry) => geometry.dispose());
    materials.forEach((material) => material.dispose());
    textures.forEach((texture) => texture.dispose());
    renderer.dispose();
  });
  canvas.hidden = false;
  fallback.hidden = true;
  controls.hidden = false;
  // Static on first load, including reduced-motion users. Animation is opt-in.
  requestRender();
}
