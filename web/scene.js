// src/progression.ts
var TERRARIUM_LEVELS = [
  { level: 1, requiredScore: 0, size: 5, organisms: 12, collection: 48 },
  { level: 2, requiredScore: 120, size: 6, organisms: 18, collection: 64 },
  { level: 3, requiredScore: 260, size: 7, organisms: 24, collection: 96 },
  { level: 4, requiredScore: 420, size: 8, organisms: 32, collection: 128 }
];
var TERRARIUM_WATER_LEVELS = TERRARIUM_LEVELS.map((level, index) => ({ ...level, requiredScore: [0, 120, 240, 360][index] }));
function expansionLevels(state) {
  return "kind" in state && state.kind === "water" ? TERRARIUM_WATER_LEVELS : TERRARIUM_LEVELS;
}
function capacity(state) {
  return expansionLevels(state).find((l) => l.level === state.progression?.level) ?? expansionLevels(state)[0];
}
function nearFacility(state, kind, x, y) {
  return state.facilities?.some((f) => f.kind === kind && f.enabled && Math.abs(f.x - x) + Math.abs(f.y - y) <= 1) ?? false;
}

// src/habitat.ts
var TERRARIUM_PLANT_RECOVERED_HEALTH = 35;
var TERRARIUM_PLANT_REQUIREMENTS = {
  fern: { minMoisture: 30, maxMoisture: 85, minLight: 0.15, maxLight: 0.7 },
  clover: { minMoisture: 20, maxMoisture: 80, minLight: 0.45, maxLight: 1 },
  moss: { minMoisture: 45, maxMoisture: 90, minLight: 0.15, maxLight: 0.4 },
  basil: { minMoisture: 30, maxMoisture: 75, minLight: 0.65, maxLight: 1 },
  bean: { minMoisture: 25, maxMoisture: 70, minLight: 0.7, maxLight: 1 },
  sunflower: { minMoisture: 25, maxMoisture: 70, minLight: 0.75, maxLight: 1 },
  mint: { minMoisture: 35, maxMoisture: 85, minLight: 0.3, maxLight: 0.85 }
};
function daylight(habitat, x, y) {
  const shaded = habitat.items.some((item) => item.id === "pebble" && Math.abs(item.x - x) + Math.abs(item.y - y) <= 1);
  let light = (0.6 + x * 0.4 / (capacity(habitat).size - 1)) * (shaded ? 0.35 : 1);
  if (nearFacility(habitat, "shade", x, y)) light *= 0.6;
  if (nearFacility(habitat, "grow_light", x, y)) light = Math.max(0.75, light);
  return light;
}
var TERRARIUM_SUNFLOWER = { seedHours: 30, seedIntervalHours: 12, seedPortions: 3 };
function sunflowerSeedRounds(growthHours) {
  return Math.max(0, Math.floor((growthHours - TERRARIUM_SUNFLOWER.seedHours) / TERRARIUM_SUNFLOWER.seedIntervalHours) + 1);
}
function hasSeedHead(plant) {
  return plant.species === "sunflower" && sunflowerSeedRounds(plant.growthHours ?? 0) > 0;
}
var TERRARIUM_FLOWER_BUD_HOURS = 12;
var TERRARIUM_FLOWER_BLOOM_HOURS = 18;
function flowerStage(plant) {
  if (plant.species !== "clover" && plant.species !== "basil" && plant.species !== "bean" && plant.species !== "sunflower" && plant.species !== "mint") return;
  if (hasSeedHead(plant)) return;
  const needs = TERRARIUM_PLANT_REQUIREMENTS[plant.species];
  if (plant.health < TERRARIUM_PLANT_RECOVERED_HEALTH || plant.dry || plant.moisture < needs.minMoisture || plant.moisture > needs.maxMoisture) return;
  if ((plant.growthHours ?? 0) >= TERRARIUM_FLOWER_BLOOM_HOURS) return "bloom";
  if ((plant.growthHours ?? 0) >= TERRARIUM_FLOWER_BUD_HOURS) return "bud";
}

// src/flight.ts
var TERRARIUM_FLY_SPECIES = ["fruit_fly", "mosquito", "moth"];
function isFlying(species) {
  return TERRARIUM_FLY_SPECIES.includes(species);
}

// ../../scratch/terrarium-0805/renderer-source/dist/types.js
function sceneSize(scene) {
  return scene.gridSize ?? 5;
}

// ../../scratch/terrarium-0805/renderer-source/dist/geometry.js
var LAND_SURFACE = { x: 84, y: 204, width: 632, height: 216 };
var WATER_SURFACE = { x: 90, y: 128, width: 620, height: 286 };
function cellPosition(surface, x, y, size) {
  return { x: surface.x + (x + 0.5) * surface.width / size, y: surface.y + (y + 0.5) * surface.height / size };
}
function plotPosition(x, y, size = 5) {
  return cellPosition(LAND_SURFACE, x, y, size);
}
function residentScale(size) {
  return 5 / size;
}

// ../../scratch/terrarium-0805/renderer-source/dist/plant-appearance.js
function blend(from, to, amount) {
  const color = (hex, offset) => Number.parseInt(hex.slice(offset, offset + 2), 16);
  return "#" + [1, 3, 5].map((offset) => Math.round(color(from, offset) * (1 - amount) + color(to, offset) * amount).toString(16).padStart(2, "0")).join("");
}
function plantAppearance(plant, night = false) {
  const stress = 1 - (plant.health ?? 100) / 100;
  const yellow = Math.min(1, stress / 0.65), brown = Math.max(0, (stress - 0.6) / 0.4);
  const age = (healthy, faded) => blend(blend(healthy, faded, yellow), "#826345", brown);
  return {
    leaf: age(night ? "#527d59" : "#65a96a", night ? "#a49b55" : "#c6b258"),
    youngLeaf: age(night ? "#65936a" : "#85bf7e", night ? "#b1a569" : "#d2c47d"),
    stem: age(night ? "#456743" : "#417e4c", "#94834e"),
    vein: age(night ? "#88aa7d" : "#c0d69b", "#e3d0a0"),
    wilt: Math.min(1, (plant.dry ? 0.55 : 0) + stress * 0.5),
    damage: plant.leafDamage ?? 0,
    spots: stress > 0.2 ? Math.min(4, Math.ceil((stress - 0.2) * 5)) : 0
  };
}
function drawLeaf(g, appearance, x, y, rx, ry, angle, index, shape = "oval") {
  g.save();
  g.translate(x, y);
  g.rotate(angle);
  const outline = () => {
    g.beginPath();
    if (shape === "oval")
      g.ellipse(0, 0, rx, ry, 0, 0, Math.PI * 2);
    else {
      g.moveTo(-rx, 0);
      g.bezierCurveTo(-rx * 0.4, -ry * 1.4, rx * 0.65, -ry, rx, 0);
      g.quadraticCurveTo(-rx * 0.1, ry * 1.45, -rx, 0);
      g.closePath();
    }
  };
  outline();
  const holes = Math.max(0, Math.min(3, Math.ceil(appearance.damage / 30) - index % 3));
  if (holes) {
    g.clip();
    outline();
    for (let i = 0; i < holes; i++) {
      const hx = rx * [-0.25, 0.22, 0.82][i], hy = ry * [0.16, -0.2, 0.25][i];
      const radius = Math.min(rx, ry) * (0.2 + appearance.damage / 700);
      g.moveTo(hx + radius, hy);
      g.arc(hx, hy, radius, 0, Math.PI * 2);
    }
  }
  g.fillStyle = index % 2 ? appearance.leaf : appearance.youngLeaf;
  g.fill("evenodd");
  if (holes || appearance.spots)
    g.clip("evenodd");
  g.strokeStyle = appearance.vein;
  g.lineWidth = 0.8;
  g.beginPath();
  g.moveTo(-rx * 0.7, 0);
  g.quadraticCurveTo(0, -ry * 0.12, rx * 0.65, 0);
  g.stroke();
  for (let i = 0; i < appearance.spots; i++) {
    g.fillStyle = i % 2 ? "#8e794b" : "#d3bb68";
    g.beginPath();
    g.ellipse(rx * [-0.38, 0.35, -0.05, 0.5][i], ry * [0.35, -0.3, -0.45, 0.25][i], Math.max(1, rx * 0.12), Math.max(0.8, ry * 0.18), i * 0.7, 0, Math.PI * 2);
    g.fill();
  }
  g.restore();
}

// ../../scratch/terrarium-0805/renderer-source/dist/neighbors.js
function drawNeighborPlant(g, plant, x, base, night, flowersOnly = false) {
  if (plant.species !== "sunflower" && plant.species !== "mint")
    return false;
  const height = 13 + plant.stage * 20;
  g.save();
  g.translate(x, base);
  g.lineCap = "round";
  const appearance = plantAppearance(plant, night);
  const stem = appearance.stem;
  g.strokeStyle = stem;
  g.lineWidth = 3 + plant.stage;
  if (plant.species === "sunflower") {
    if (!flowersOnly) {
      g.beginPath();
      g.moveTo(0, 0);
      g.quadraticCurveTo(-4, -height * 0.5, 0, -height);
      g.stroke();
      for (let i = 0; i <= plant.stage; i++) {
        const side = i % 2 ? -1 : 1, cy = -10 - i * 15;
        drawLeaf(g, appearance, side * 17, cy - 5 + appearance.wilt * 5, 19, 11, -side * (0.35 - appearance.wilt * 0.7), i, "pointed");
      }
    }
    if (plant.seedHead) {
      g.save();
      g.translate(3, -height + 2);
      g.rotate(0.18);
      g.fillStyle = "#ad8744";
      g.beginPath();
      g.arc(0, 0, 19, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = night ? "#544737" : "#6e5036";
      g.beginPath();
      g.arc(0, 0, 15, 0, Math.PI * 2);
      g.fill();
      for (let i = 0; i < 21; i++) {
        const a = i * 2.39996, r = Math.sqrt(i / 21) * 12;
        g.save();
        g.translate(Math.cos(a) * r, Math.sin(a) * r);
        g.rotate(a);
        g.fillStyle = i % 3 ? "#d5bd80" : "#f0dca8";
        g.beginPath();
        g.ellipse(0, 0, 1.5, 2.3, 0, 0, Math.PI * 2);
        g.fill();
        g.restore();
      }
      g.restore();
    } else if (plant.flower) {
      g.save();
      g.translate(0, -height);
      if (plant.flower === "bud") {
        g.fillStyle = "#82a44f";
        g.beginPath();
        g.ellipse(0, 0, 9, 12, -0.15, 0, Math.PI * 2);
        g.fill();
        g.strokeStyle = "#b5c878";
        g.lineWidth = 1;
        g.beginPath();
        g.moveTo(0, 9);
        g.lineTo(0, -10);
        g.stroke();
      } else {
        for (let ring = 0; ring < 2; ring++)
          for (let i = 0; i < 13; i++) {
            const a = i * Math.PI * 2 / 13 + ring * 0.2;
            g.fillStyle = night ? ["#bea552", "#d2b963"][ring] : ["#e7ab3b", "#f5ca59"][ring];
            g.beginPath();
            g.ellipse(Math.cos(a) * 17, Math.sin(a) * 17, 12 - ring * 2, 4.7, a, 0, Math.PI * 2);
            g.fill();
          }
        g.fillStyle = "#705036";
        g.beginPath();
        g.arc(0, 0, 12, 0, Math.PI * 2);
        g.fill();
        g.fillStyle = "#b58c50";
        for (let i = 0; i < 13; i++) {
          const a = i * 2.39996, r = Math.sqrt(i / 13) * 8;
          g.beginPath();
          g.arc(Math.cos(a) * r, Math.sin(a) * r, 1, 0, Math.PI * 2);
          g.fill();
        }
      }
      g.restore();
    }
  } else {
    for (const side of plant.stage > 1 ? [-1, 0, 1] : [0]) {
      const top = -height * (side ? 0.78 : 1), endX = side * 15;
      if (!flowersOnly) {
        g.strokeStyle = stem;
        g.lineWidth = 2.5;
        g.beginPath();
        g.moveTo(0, 0);
        g.quadraticCurveTo(endX, top * 0.4, endX, top);
        g.stroke();
        for (let level = 0; level <= plant.stage; level++)
          for (const direction of [-1, 1]) {
            const cy = -8 + top * (0.18 + level * 0.2), cx = endX * Math.min(1, cy / top * 1.3);
            const tipX = cx + direction * (13 + plant.stage * 1.5), tipY = cy - 9;
            drawLeaf(g, appearance, (cx + tipX) / 2, (cy + tipY) / 2 + appearance.wilt * 4, 10 + plant.stage, 6, -direction * (0.4 - appearance.wilt * 0.8), level + (side + 1) * 4 + (direction + 1) / 2, "pointed");
          }
      }
      if (plant.flower)
        for (let i = 0; i < 4; i++)
          for (const direction of [-1, 1]) {
            g.fillStyle = plant.flower === "bud" ? "#a79cc1" : night ? "#b8abc9" : "#d4bde5";
            g.beginPath();
            g.ellipse(endX + direction * 4, top - i * 5, 3, plant.flower === "bud" ? 3 : 2, direction * 0.4, 0, Math.PI * 2);
            g.fill();
          }
    }
  }
  g.restore();
  return true;
}
function drawLadybug(g, x, y, night) {
  g.save();
  g.translate(x, y);
  g.strokeStyle = "#393c37";
  g.lineWidth = 1.5;
  g.lineCap = "round";
  for (const side of [-1, 1])
    for (const dy of [-5, 0, 5]) {
      g.beginPath();
      g.moveTo(side * 7, dy);
      g.lineTo(side * 12, dy + 3);
      g.stroke();
    }
  g.fillStyle = "#343d36";
  g.beginPath();
  g.ellipse(0, -7, 6, 5, 0, 0, Math.PI * 2);
  g.fill();
  g.fillStyle = night ? "#b75642" : "#dc6551";
  g.beginPath();
  g.ellipse(0, 1, 10, 10, 0, 0, Math.PI * 2);
  g.fill();
  g.strokeStyle = "#813f35";
  g.lineWidth = 1.3;
  g.beginPath();
  g.moveTo(0, -8);
  g.lineTo(0, 10);
  g.stroke();
  g.fillStyle = "#343d36";
  for (const [dx, dy, r] of [[-4, -2, 2], [4, -2, 2], [-5, 4, 2.2], [5, 4, 2.2], [0, 7, 1.6]]) {
    g.beginPath();
    g.arc(dx, dy, r, 0, Math.PI * 2);
    g.fill();
  }
  g.fillStyle = "#fff0d3";
  g.beginPath();
  g.ellipse(-4, -4, 2, 3, 0.6, 0, Math.PI * 2);
  g.fill();
  for (const dx of [-2.5, 2.5]) {
    g.beginPath();
    g.arc(dx, -9, 1.2, 0, Math.PI * 2);
    g.fill();
  }
  g.restore();
}

// ../../scratch/terrarium-0805/renderer-source/dist/flowers.js
function drawPlantFlowers(g, plant, x, y, night) {
  const stage = plant.flower;
  if (!stage && !plant.seedHead)
    return;
  if (drawNeighborPlant(g, { ...plant, stage: 3, dry: false }, x, y + 73, night, true))
    return;
  g.save();
  g.translate(x, y);
  g.lineCap = "round";
  const stem = (tx, ty) => {
    g.strokeStyle = "#568451";
    g.lineWidth = 2;
    g.beginPath();
    g.moveTo(0, 8);
    g.quadraticCurveTo(tx * 0.35, ty * 0.6, tx, ty);
    g.stroke();
  };
  const petal = (px, py, rx, ry, angle, color) => {
    g.fillStyle = color;
    g.beginPath();
    g.ellipse(px, py, rx, ry, angle, 0, Math.PI * 2);
    g.fill();
  };
  const bud = (px, py, color) => {
    petal(px, py, 4, 7, -0.15, color);
    g.fillStyle = "#5f965a";
    g.beginPath();
    g.moveTo(px - 5, py + 3);
    g.lineTo(px, py + 7);
    g.lineTo(px + 5, py + 2);
    g.lineTo(px, py + 4);
    g.fill();
  };
  if (plant.species === "clover") {
    for (const [tx, ty, scale] of [[-16, -19, 0.8], [11, -34, 1], [29, -9, 0.7]]) {
      stem(tx, ty + 8);
      g.save();
      g.translate(tx, ty);
      g.scale(scale, scale);
      if (stage === "bud" || night)
        bud(0, 0, "#d49cb4");
      else {
        petal(0, 2, 13, 11, 0, "#c47b9a");
        for (let ring = 0; ring < 3; ring++) {
          const count = 12 - ring * 3, radius = 9 - ring * 3;
          for (let i = 0; i < count; i++) {
            const angle = i * Math.PI * 2 / count + ring * 0.4;
            petal(Math.cos(angle) * radius, Math.sin(angle) * radius * 0.8, 2.6, 5, angle + Math.PI / 2, ["#edb4c8", "#f7cfda", "#fff0e9"][ring]);
          }
        }
      }
      g.restore();
    }
  } else if (plant.species === "basil") {
    stem(0, -49);
    for (let level = 0; level < 5; level++)
      for (const side of [-1, 1]) {
        const px = side * (8 - level), py = -7 - level * 9;
        stem(px, py + 2);
        if (stage === "bud" || night)
          bud(px, py, "#b7a8ce");
        else {
          petal(px, py, 5, 4, side * 0.6, "#c8b5de");
          petal(px + side * 2, py + 2, 5, 3, -side * 0.25, "#f8efff");
          petal(px - side, py - 2, 3, 4, side * 0.3, "#eee4f7");
          petal(px + side * 3, py + 1, 1.5, 1.5, 0, "#d3a56b");
        }
      }
    bud(0, -51, "#c3b7d4");
  } else {
    for (const [tx, ty] of [[-15, -18], [20, -32], [31, -5]]) {
      stem(tx, ty + 6);
      if (stage === "bud" || night)
        bud(tx, ty, "#d69ab1");
      else {
        petal(tx, ty - 4, 12, 10, -0.15, "#d88fac");
        petal(tx, ty - 7, 9, 7, -0.15, "#f7cbd9");
        petal(tx - 7, ty + 3, 9, 5, 0.6, "#e9a6c0");
        petal(tx + 7, ty + 3, 9, 5, -0.6, "#f0b7ce");
        petal(tx + 1, ty + 5, 5, 6, -0.2, "#fff0e9");
        petal(tx + 1, ty + 2, 2, 2, 0, "#d6ab65");
      }
    }
  }
  g.restore();
}

// ../../scratch/terrarium-0805/renderer-source/dist/nest-viewer.js
var NEST_SIZE = 4;
var STYLE = `
.tn-dialog{box-sizing:border-box;width:min(720px,calc(100vw - 24px));max-height:calc(100dvh - 32px);padding:0;border:1px solid #c0ac85;border-radius:24px;background:#f7f1e4;color:#473c2e;box-shadow:0 30px 120px #17170e55;font:14px/1.5 system-ui,sans-serif;overflow:auto}
.tn-dialog::backdrop{background:#18251dd0;backdrop-filter:blur(7px)}.tn-head{padding:22px 24px 12px;display:flex;justify-content:space-between;gap:16px;align-items:center}.tn-head h2{font-size:22px;margin:0;font-weight:600}.tn-kicker{font:10px monospace;letter-spacing:2px;color:#8c7b5f;margin-bottom:5px}
.tn-dialog{overscroll-behavior:contain}.tn-head{position:sticky;top:0;z-index:2;background:#f7f1e4}
.tn-close,.tn-open{font:inherit;cursor:pointer;border:1px solid #b6aa88;border-radius:24px;background:#fffaf0;color:#4c573a;padding:8px 15px;transition:background .2s,transform .2s}.tn-close:hover,.tn-open:hover{background:#e8e4d1;transform:translateY(-1px)}.tn-toolbar{position:relative;display:flex;gap:6px;flex-wrap:wrap;z-index:1}.scene-wrap>.tn-toolbar{position:absolute;left:12px;bottom:10px}.tn-toolbar[hidden]{display:none}.tn-open{font-size:11px;padding:5px 11px;background:#fff9e8e8;backdrop-filter:blur(8px)}
.tn-summary{display:flex;flex-wrap:wrap;gap:8px;padding:0 24px 16px}.tn-summary span{background:#e9e2cd;border-radius:8px;padding:6px 10px;font-size:12px;font-variant-numeric:tabular-nums}
.tn-grid{display:grid;grid-template-columns:repeat(var(--nest-size,4),minmax(80px,1fr));gap:9px;padding:18px;margin:0 24px;overflow-x:auto;background:#796448;border:1px solid #6b593f;border-radius:18px;box-shadow:inset 0 3px 15px #3d2d2030}
.tn-cell{position:relative;box-sizing:border-box;width:100%;min-width:0;min-height:80px;aspect-ratio:1;overflow:hidden;border:1px solid #c6ac78;border-radius:19% 28% 23% 26%;background:radial-gradient(ellipse at 40% 35%,#a68a5e,#967347);color:#fff2d2;cursor:pointer;box-shadow:inset 0 3px 10px #3f2b2030,0 2px 2px #30251d28;font:inherit;padding:8px;transition:background .2s,box-shadow .2s}
.tn-cell:hover,.tn-cell[aria-pressed=true]{background:radial-gradient(ellipse at 40% 35%,#b79b6a,#a7804c);box-shadow:inset 0 0 0 2px #f3dba5,0 2px 8px #30251d30}.tn-cell:focus-visible,.tn-close:focus-visible,.tn-open:focus-visible{outline:3px solid #9cbd9a;outline-offset:3px}
.tn-cell-label{position:absolute;left:9px;top:6px;font-size:10px;color:#f6e7c3c9}.tn-occupants{position:absolute;inset:38px 5px 20px;display:flex;justify-content:center;align-items:center;gap:6px;overflow:hidden}.tn-ant{display:inline-flex;flex:0 1 auto;min-width:0;width:40px;height:30px;--ant-abdomen:#b77a50;--ant-head:#d7a171;--ant-chest:#c48a5c}.tn-ant svg{display:block;width:100%;height:100%;overflow:visible}.tn-ant.queen{width:46px;height:34px;--ant-abdomen:#c6984f;--ant-head:#e3b97e;--ant-chest:#d2a164}.tn-eggs{display:flex;flex:0 0 16px;flex-wrap:wrap;align-content:center;justify-content:center;gap:2px}.tn-egg{display:inline-block;width:6px;height:8px;border-radius:60% 50% 55% 45%;background:#fff1ce;box-shadow:inset -2px -1px 1px #d4bd91;transform:rotate(25deg)}.tn-food{flex:0 0 auto;white-space:nowrap;font-size:13px;color:#ffe2a1;font-variant-numeric:tabular-nums}.tn-cell-count{position:absolute;bottom:5px;right:8px;font-size:10px;color:#fff0ce}.tn-detail{padding:16px 24px 22px;min-height:65px}.tn-detail h3{font-size:13px;margin:0 0 5px}.tn-detail p{font-size:12px;margin:3px 0;color:#7b6a50;overflow-wrap:anywhere}.tn-note{padding:0 24px 18px;font-size:12px;color:#85755b}
@media(max-width:480px){.tn-head{padding:16px 16px 12px}.tn-head h2{font-size:19px}.tn-grid{padding:10px;margin:0 12px;gap:6px}.tn-cell{padding:5px}.tn-cell-label{font-size:8px;left:5px}.tn-occupants{gap:3px;inset:38px 3px 18px}.tn-summary{padding:0 16px 12px;gap:5px}.tn-summary span{font-size:11px;padding:4px 7px}.tn-detail{padding:14px 16px}.tn-cell-count{font-size:8px;right:5px}.tn-ant{width:34px;height:26px}.tn-ant.queen{width:40px;height:30px}.tn-ant~.tn-ant{display:none}.tn-eggs{flex-basis:12px}.tn-egg{width:5px;height:7px}.tn-food{font-size:10px}}
@media(prefers-reduced-motion:reduce){.tn-cell,.tn-open,.tn-close{transition:none}}
.tn-cell.has-queen{border-color:#ffdd8a;box-shadow:inset 0 0 0 1px #edce8d,0 0 9px #f8d87b35}.tn-queen-badge{position:absolute;right:6px;top:20px;border:1px solid #ffe8ab;border-radius:5px;padding:1px 4px;background:#f3d58b;color:#563d1c;font-size:10px;font-weight:600;line-height:1.3}.tn-cell-count{left:3px;right:3px;text-align:center;white-space:nowrap}.tn-time{font-variant-numeric:tabular-nums}
@media(max-width:480px){.tn-cell-label{font-size:9px}.tn-cell-count,.tn-queen-badge{font-size:9px}}
@media(max-width:360px){.tn-ant{width:24px;height:18px}.tn-ant.queen{width:28px;height:21px}.tn-eggs{flex-basis:10px;gap:1px}.tn-egg{width:4px;height:6px}}
@media(pointer:coarse){.tn-open,.tn-close{min-height:44px;padding:9px 14px}}
:root:is([data-theme="dark"],[data-color-mode="dark"]) .tn-dialog{background:#23251d;color:#e4e3ce;border-color:#65634b}
:root:is([data-theme="dark"],[data-color-mode="dark"]) .tn-head{background:#23251d}
:root:is([data-theme="dark"],[data-color-mode="dark"]) :is(.tn-kicker,.tn-detail p){color:#b5b299}
:root:is([data-theme="dark"],[data-color-mode="dark"]) :is(.tn-close,.tn-open){background:#313b29;color:#d0ddbd;border-color:#697655}
:root:is([data-theme="dark"],[data-color-mode="dark"]) :is(.tn-close,.tn-open):hover{background:#405135}
:root:is([data-theme="dark"],[data-color-mode="dark"]) .tn-summary span{background:#393d2e}
:root:is([data-theme="dark"],[data-color-mode="dark"]) .tn-grid{background:#403928;border-color:#62563b}
:root:is([data-theme="dark"],[data-color-mode="dark"]) .tn-cell{background:radial-gradient(ellipse at 40% 35%,#726443,#574329);border-color:#8c794d}
:root:is([data-theme="dark"],[data-color-mode="dark"]) .tn-cell:is(:hover,[aria-pressed="true"]){background:radial-gradient(ellipse at 40% 35%,#88764e,#6a5230)}

`;
var ANT_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 36" fill="none" focusable="false">
  <ellipse cx="23" cy="32" rx="17" ry="1.8" fill="#60432b" opacity=".15"/>
  <g stroke="#9c7152" stroke-width="2.3" stroke-linecap="round" stroke-linejoin="round">
    <path d="M15 19Q10 21 10 26M22 19Q21 22 22 26M28 19Q32 21 33 25"/>
  </g>
  <g stroke="#80543d" stroke-width="1.8" stroke-linecap="round">
    <path d="M31 9Q29 3 26 5M36 9Q38 3 41 5"/>
  </g>
  <g stroke="#81553b" stroke-width="1.1">
    <ellipse cx="13" cy="20" rx="9" ry="7" fill="var(--ant-abdomen)"/>
    <ellipse cx="23" cy="21" rx="5" ry="4.8" fill="var(--ant-chest)"/>
    <ellipse cx="33" cy="15" rx="8" ry="7.8" fill="var(--ant-head)"/>
  </g>
  <ellipse cx="10" cy="17" rx="3.8" ry="1.7" transform="rotate(-24 10 17)" fill="#ffe4b2" opacity=".45"/>
  <path d="M17 24Q16 29 12 29M23 25Q23 30 20 30M28 23Q31 28 34 28" stroke="#80543d" stroke-width="2.3" stroke-linecap="round"/>
  <g fill="#fff6df"><ellipse cx="30" cy="13.5" rx="2.1" ry="2.4"/><ellipse cx="36" cy="13.5" rx="2.1" ry="2.4"/></g>
  <g fill="#594132"><circle cx="30.6" cy="13.8" r="1.15"/><circle cx="36.6" cy="13.8" r="1.15"/></g>
  <g fill="#fffaf0"><circle cx="30.9" cy="13.2" r=".45"/><circle cx="36.9" cy="13.2" r=".45"/></g>
  <ellipse cx="28" cy="18" rx="1.7" ry="1" fill="#d98c70" opacity=".55"/>
  <path d="M32 18Q34 20.5 36 18" stroke="#81553b" stroke-width="1.1" stroke-linecap="round"/>
</svg>`;
function createNestViewer(canvas, language = "zh") {
  const doc = canvas.ownerDocument;
  if (!doc.getElementById("terrarium-nest-style")) {
    const style = doc.createElement("style");
    style.id = "terrarium-nest-style";
    style.textContent = STYLE;
    doc.head.append(style);
  }
  const zh = language === "zh";
  const text = zh ? { title: "\u8681\u5DE2\u5185\u90E8", enter: "\u8FDB\u5165\u8681\u5DE2", close: "\u8FD4\u56DE\u751F\u6001\u7BB1", ant: "\u8682\u8681", egg: "\u5375", queen: "\u8681\u540E", worker: "\u5DE5\u8681", food: "\u50A8\u7CAE", entrance: "\u5165\u53E3", storage: "\u7CAE\u4ED3", nursery: "\u80B2\u5E7C\u5BA4", room: "\u5DE2\u5BA4", empty: "\u6B64\u683C\u6CA1\u6709\u8682\u8681\u6216\u5375\u3002", legacy: "\u8BE5\u5BA2\u6237\u7AEF\u5C1A\u672A\u4E0A\u62A5\u8681\u5DE2\u5185\u90E8\u72B6\u6001\uFF1B\u5347\u7EA7 World \u540E\u53EF\u67E5\u770B\u8681\u540E\u4E0E\u5DE2\u5185\u751F\u7269\u3002" } : { title: "Inside the nest", enter: "Enter nest", close: "Back to terrarium", ant: "Ants", egg: "Eggs", queen: "Queen", worker: "Worker", food: "Food", entrance: "Entrance", storage: "Pantry", nursery: "Nursery", room: "Chamber", empty: "No ants or eggs in this cell.", legacy: "This client has not reported colony locations. Upgrade the World to see the queen and nest residents." };
  const el = (tag, cls, label) => {
    const node2 = doc.createElement(tag);
    node2.className = cls;
    if (label !== void 0)
      node2.textContent = label;
    return node2;
  };
  const toolbar = el("div", "tn-toolbar"), dialog = el("dialog", "tn-dialog");
  const head = el("div", "tn-head"), title = el("h2", "", text.title), close = el("button", "tn-close", text.close);
  close.type = "button";
  const heading = el("div", "");
  const kicker = el("div", "tn-kicker", "COLONY / 4 \xD7 4");
  heading.append(kicker, title);
  head.append(heading, close);
  const summary = el("div", "tn-summary");
  summary.setAttribute("aria-live", "polite");
  const grid = el("div", "tn-grid");
  grid.setAttribute("role", "group");
  grid.setAttribute("aria-label", zh ? "4\xD74 \u8681\u5DE2\u683C\u5B50" : "4 by 4 nest cells");
  const detail = el("div", "tn-detail");
  detail.setAttribute("aria-live", "polite");
  const note = el("p", "tn-note", text.legacy);
  dialog.append(head, summary, grid, detail, note);
  doc.body.append(dialog);
  dialog.setAttribute("aria-label", text.title);
  let state = null, selectedId = null, cellIndex = 10, recordedAtMs;
  const triggers = /* @__PURE__ */ new Map();
  let returnView = null;
  const makeCell = (index) => {
    const cell = el("button", "tn-cell");
    cell.type = "button";
    cell.addEventListener("click", () => {
      cellIndex = index;
      render();
    });
    return cell;
  };
  let cells = [];
  const roomName = (x, y) => x === 0 && y === 0 ? text.entrance : x === 3 && y === 0 ? text.storage : x === 2 && y === 2 ? text.nursery : text.room;
  const remainingTime = (hatchesAtMs) => {
    const minutes = Math.ceil((hatchesAtMs - (recordedAtMs ?? Date.now())) / 6e4);
    if (minutes <= 0)
      return zh ? "\u7B49\u5F85\u5B75\u5316" : "Awaiting hatching";
    const hours = Math.floor(minutes / 60), rest = minutes % 60;
    const duration = zh ? `${hours ? `${hours} \u5C0F\u65F6` : ""}${rest ? `${hours ? " " : ""}${rest} \u5206\u949F` : ""}` : `${hours ? `${hours}h` : ""}${rest ? `${hours ? " " : ""}${rest}m` : ""}`;
    return zh ? `\u9884\u8BA1 ${duration}\u540E\u5B75\u5316` : `Hatches in about ${duration}`;
  };
  const render = () => {
    const nest = state?.nests?.find((nest2) => nest2.id === selectedId);
    if (!state || !nest) {
      if (dialog.open)
        dialog.close();
      return;
    }
    const size = nest.size ?? NEST_SIZE;
    dialog.style.width = size > 5 ? "min(920px,calc(100vw - 24px))" : "min(720px,calc(100vw - 24px))";
    if (cells.length !== size * size) {
      cells = Array.from({ length: size * size }, (_, i) => makeCell(i));
      grid.replaceChildren(...cells);
    }
    cellIndex = Math.min(cellIndex, size * size - 1);
    grid.style.setProperty("--nest-size", String(size));
    grid.setAttribute("aria-label", zh ? `${size}\xD7${size} \u8681\u5DE2\u683C\u5B50` : `${size} by ${size} nest cells`);
    kicker.textContent = `COLONY / ${size} \xD7 ${size}`;
    const ants = state.bugs.filter((b) => b.species === "ant" && b.inside?.nestId === nest.id);
    const eggs = (state.eggs ?? []).filter((e) => e.species === "ant" && e.inside?.nestId === nest.id);
    const contents = { ants, eggs, queens: ants.filter((a) => a.role === "queen").length, food: nest.food };
    title.textContent = `${text.title} \xB7 ${nest.id}`;
    summary.replaceChildren(...[`${size}\xD7${size} \xB7 ${size * size} ${zh ? "\u683C" : "cells"}`, `${zh ? "\u5165\u53E3" : "Entrances"} ${1 + (nest.entrances?.length ?? 0)}`, `${text.ant} ${contents.ants.length}`, `${text.queen} ${contents.queens}`, `${text.egg} ${contents.eggs.length}`, `${text.food} ${contents.food}/${nest.foodCapacity ?? "\u2014"}`].map((label) => el("span", "", label)));
    note.hidden = state.bugs.some((bug) => bug.species === "ant" && bug.role !== void 0);
    for (const [index, cell] of cells.entries()) {
      const x = index % size, y = Math.floor(index / size);
      const ants2 = contents.ants.filter((ant) => ant.inside.x === x && ant.inside.y === y), eggs2 = contents.eggs.filter((egg) => egg.inside.x === x && egg.inside.y === y);
      const hasQueen = ants2.some((ant) => ant.role === "queen");
      cell.classList.toggle("has-queen", hasQueen);
      cell.setAttribute("aria-pressed", String(index === cellIndex));
      cell.setAttribute("aria-label", `${roomName(x, y)} (${x},${y}), ${text.ant} ${ants2.length}, ${text.egg} ${eggs2.length}${hasQueen ? `, ${text.queen}` : ""}`);
      const occupants = el("span", "tn-occupants");
      const visibleAnts = [...ants2].sort((a, b) => Number(b.role === "queen") - Number(a.role === "queen")).slice(0, 2);
      for (const ant of visibleAnts) {
        const icon = el("span", "tn-ant" + (ant.role === "queen" ? " queen" : ""));
        icon.innerHTML = ANT_SVG;
        icon.title = `${ant.id} \xB7 ${ant.role === "queen" ? text.queen : text.worker}`;
        icon.setAttribute("aria-hidden", "true");
        occupants.append(icon);
      }
      if (eggs2.length) {
        const cluster = el("span", "tn-eggs");
        cluster.setAttribute("aria-hidden", "true");
        for (let i = 0; i < Math.min(3, eggs2.length); i++)
          cluster.append(el("span", "tn-egg"));
        occupants.append(cluster);
      }
      if (x === 3 && y === 0 && nest.food)
        occupants.append(el("span", "tn-food", `\u25CF ${nest.food}`));
      cell.replaceChildren(el("span", "tn-cell-label", `${roomName(x, y)} ${x},${y}`), occupants, el("span", "tn-cell-count", `${zh ? "\u8681" : "Ant"} ${ants2.length} \xB7 ${zh ? "\u5375" : "Egg"} ${eggs2.length}`));
      if (hasQueen)
        cell.append(el("span", "tn-queen-badge", text.queen));
      if (index === cellIndex) {
        detail.replaceChildren(el("h3", "", `${roomName(x, y)} (${x},${y})`));
        if (!ants2.length && !eggs2.length)
          detail.append(el("p", "", text.empty));
        if (x === 3 && y === 0)
          detail.append(el("p", "", `${text.food}: ${nest.food}`));
        for (const ant of ants2)
          detail.append(el("p", "", `${ant.id} \xB7 ${ant.role === "queen" ? text.queen : text.worker}${ant.energy === void 0 ? "" : ` \xB7 ${zh ? "\u4F53\u529B" : "Energy"} ${ant.energy.toFixed(0)}`}${ant.hunger === void 0 ? "" : ` \xB7 ${zh ? "\u9965\u997F" : "Hunger"} ${ant.hunger.toFixed(0)}/100`}`));
        for (const egg of eggs2) {
          const row = el("p", "", `${egg.id} \xB7 ${remainingTime(egg.hatchesAtMs)} \xB7 `);
          const time = el("time", "tn-time", new Date(egg.hatchesAtMs).toLocaleString(zh ? "zh-CN" : "en"));
          time.dateTime = new Date(egg.hatchesAtMs).toISOString();
          row.append(time);
          detail.append(row);
        }
      }
    }
  };
  const restoreView = () => {
    if (dialog.open || !returnView)
      return;
    const saved = returnView;
    returnView = null;
    if (saved.opener.isConnected)
      saved.opener.focus({ preventScroll: true });
    for (const { node: node2, x, y } of saved.ancestors)
      if (node2.isConnected)
        node2.scrollTo({ left: x, top: y, behavior: "instant" });
    doc.defaultView.scrollTo({ left: saved.x, top: saved.y, behavior: "instant" });
  };
  const open = (nest, opener) => {
    selectedId = nest.id;
    render();
    if (dialog.open)
      return;
    const ancestors = [];
    for (let node2 = canvas.parentElement; node2; node2 = node2.parentElement)
      ancestors.push({ node: node2, x: node2.scrollLeft, y: node2.scrollTop });
    returnView = { opener, x: doc.defaultView.scrollX, y: doc.defaultView.scrollY, ancestors };
    opener.focus({ preventScroll: true });
    dialog.showModal();
    if (!matchMedia("(prefers-reduced-motion: reduce)").matches)
      dialog.animate([{ opacity: 0, transform: "translateY(16px) scale(.98)" }, { opacity: 1, transform: "none" }], { duration: 240, easing: "ease-out" });
  };
  const hit = (event) => {
    const bounds = canvas.getBoundingClientRect();
    const rx = Math.max(24, 30 * bounds.width / canvas.width), ry = Math.max(24, 20 * bounds.height / canvas.height);
    const targets = (state?.nests ?? []).flatMap((nest2) => [nest2, ...nest2.entrances ?? []].map((entrance) => ({
      nest: nest2,
      dx: event.clientX - bounds.left - plotPosition(entrance.x, entrance.y, sceneSize(state)).x * bounds.width / canvas.width,
      dy: event.clientY - bounds.top - plotPosition(entrance.x, entrance.y, sceneSize(state)).y * bounds.height / canvas.height
    }))).filter(({ dx, dy }) => Math.abs(dx) <= rx && Math.abs(dy) <= ry);
    targets.sort((a, b) => a.dx ** 2 + a.dy ** 2 - b.dx ** 2 - b.dy ** 2);
    const nest = targets[0]?.nest;
    if (nest)
      open(nest, triggers.get(nest.id) ?? canvas);
  };
  dialog.addEventListener("close", restoreView);
  close.addEventListener("click", () => dialog.close());
  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) {
      const r = dialog.getBoundingClientRect();
      if (event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom)
        dialog.close();
    }
  });
  canvas.addEventListener("click", hit);
  let nestIds = "";
  return {
    setState(next, atMs) {
      state = next;
      recordedAtMs = atMs;
      const nests = next.nests ?? [];
      if (canvas.parentElement && toolbar.parentElement !== canvas.parentElement)
        canvas.after(toolbar);
      const ids = nests.map((nest) => nest.id).join(",");
      if (ids !== nestIds) {
        nestIds = ids;
        triggers.clear();
        toolbar.replaceChildren(...nests.map((nest) => {
          const button = el("button", "tn-open", `${text.enter} \xB7 ${nest.id}`);
          button.type = "button";
          triggers.set(nest.id, button);
          button.addEventListener("click", () => {
            const current = state?.nests?.find((current2) => current2.id === nest.id);
            if (current)
              open(current, button);
          });
          return button;
        }));
      }
      toolbar.hidden = !nests.length;
      canvas.style.cursor = nests.length ? "pointer" : "";
      if (dialog.open)
        render();
    },
    dispose() {
      canvas.removeEventListener("click", hit);
      canvas.style.cursor = "";
      if (dialog.open)
        dialog.close();
      restoreView();
      dialog.removeEventListener("close", restoreView);
      toolbar.remove();
      dialog.remove();
    }
  };
}

// ../../scratch/terrarium-0805/renderer-source/dist/nest-scene.js
function drawAntNest(g, x, y, night) {
  g.save();
  g.fillStyle = "rgba(35,25,18,.2)";
  g.beginPath();
  g.ellipse(x + 2, y + 5, 24, 6, 0, 0, Math.PI * 2);
  g.fill();
  const mound = g.createLinearGradient(0, y - 9, 0, y + 6);
  mound.addColorStop(0, night ? "#a08b67" : "#dfbf87");
  mound.addColorStop(1, night ? "#67543b" : "#aa7e4e");
  g.fillStyle = mound;
  g.beginPath();
  g.moveTo(x - 23, y + 2);
  g.bezierCurveTo(x - 16, y, x - 14, y - 9, x - 3, y - 8);
  g.bezierCurveTo(x + 10, y - 10, x + 16, y - 1, x + 23, y + 3);
  g.bezierCurveTo(x + 12, y + 9, x - 16, y + 8, x - 23, y + 2);
  g.fill();
  g.strokeStyle = night ? "#b09a72" : "#f0d49b";
  g.lineWidth = 1.5;
  g.beginPath();
  g.ellipse(x - 1, y - 1, 10, 4.5, -0.1, 0, Math.PI * 2);
  g.stroke();
  g.fillStyle = night ? "#1c1b18" : "#30251e";
  g.beginPath();
  g.ellipse(x - 1, y, 8.5, 3.5, -0.1, 0, Math.PI * 2);
  g.fill();
  for (let pebble = 0; pebble < 9; pebble++) {
    const angle = pebble * 2.4;
    g.fillStyle = pebble % 2 ? night ? "#7f6b4e" : "#b48955" : night ? "#af966d" : "#edce91";
    g.beginPath();
    g.ellipse(x + Math.cos(angle) * (13 + pebble % 3 * 2), y + 2 + Math.sin(angle) * 3.5, 1.5, 0.9, angle, 0, Math.PI * 2);
    g.fill();
  }
  g.restore();
}

// ../../scratch/terrarium-0805/renderer-source/dist/scene.js
function bugPositions(bugs, positions = /* @__PURE__ */ new Map(), size = 5) {
  const groups = /* @__PURE__ */ new Map();
  for (const bug of bugs) {
    const tile = `${bug.x},${bug.y}`;
    const group = groups.get(tile) ?? [];
    group.push(bug.id);
    groups.set(tile, group);
  }
  for (const group of groups.values())
    group.sort();
  const result = /* @__PURE__ */ new Map();
  for (const bug of bugs) {
    const position = positions.get(bug.id) ?? bug;
    const group = groups.get(`${bug.x},${bug.y}`);
    const ordinal = group.indexOf(bug.id);
    const columns = Math.min(3, group.length);
    const row = Math.floor(ordinal / columns);
    const rows = Math.ceil(group.length / columns);
    const rowSize = Math.min(columns, group.length - row * columns);
    const spacing = residentScale(size);
    const plot = plotPosition(position.x, position.y, size);
    result.set(bug.id, { x: plot.x + (ordinal % columns - (rowSize - 1) / 2) * 28 * spacing, y: plot.y - 6 * spacing + (row - (rows - 1) / 2) * Math.min(19 * spacing, 24 / rows) });
  }
  return result;
}
function drawScene(canvas, state, positions = /* @__PURE__ */ new Map(), hits = []) {
  const g = canvas.getContext("2d");
  if (!g)
    return;
  const size = sceneSize(state);
  const scale = residentScale(size);
  const plot = (x, y) => plotPosition(x, y, size);
  const night = state.phase === "night";
  const sky = g.createLinearGradient(0, 0, 0, 500);
  sky.addColorStop(0, night ? "#162c47" : "#9ccbcf");
  sky.addColorStop(1, night ? "#657b80" : "#e1e9bd");
  g.fillStyle = sky;
  g.fillRect(0, 0, 800, 500);
  g.fillStyle = night ? "#e8e7ce" : "#fff1a7";
  g.beginPath();
  g.arc(680, 83, night ? 23 : 32, 0, Math.PI * 2);
  g.fill();
  if (night) {
    g.fillStyle = "#d4e0e5";
    for (const [x, y] of [[65, 67], [151, 111], [307, 68], [460, 89], [570, 49], [741, 151]]) {
      g.beginPath();
      g.arc(x, y, 2, 0, Math.PI * 2);
      g.fill();
    }
  }
  g.fillStyle = night ? "#415866" : "#91af91";
  g.beginPath();
  g.ellipse(400, 453, 350, 32, 0, 0, Math.PI * 2);
  g.fill();
  g.fillStyle = night ? "rgba(99,136,147,.2)" : "rgba(236,255,241,.25)";
  g.beginPath();
  g.roundRect(60, 70, 680, 400, 24);
  g.fill();
  g.fillStyle = night ? "#50493e" : "#715438";
  g.beginPath();
  g.roundRect(76, 202, 648, 250, [10, 10, 20, 20]);
  g.fill();
  g.fillStyle = night ? "#756b53" : "#a7875b";
  g.beginPath();
  g.roundRect(76, 194, 648, 236, 12);
  g.fill();
  const cellWidth = LAND_SURFACE.width / size, cellHeight = LAND_SURFACE.height / size;
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) {
      const center = plot(x, y);
      const cell = state.soil?.find((cell2) => cell2.x === x && cell2.y === y);
      const left = center.x - cellWidth / 2, top = center.y - cellHeight / 2;
      g.fillStyle = `rgba(36,91,107,${(cell?.moisture ?? 0) / 230})`;
      g.fillRect(left + 1, top + 1, cellWidth - 2, cellHeight - 2);
      if ((cell?.daylight ?? 1) < 0.4) {
        g.fillStyle = "rgba(20,30,40,.18)";
        g.fillRect(left + 1, top + 1, cellWidth - 2, cellHeight - 2);
      }
      g.strokeStyle = night ? "#dce9c51c" : "#f9efd42e";
      g.lineWidth = 1;
      g.beginPath();
      g.rect(left, top, cellWidth, cellHeight);
      g.stroke();
    }
  for (const colony of state.nests ?? [])
    for (const nest of [colony, ...colony.entrances ?? []]) {
      const { x, y } = plot(nest.x, nest.y);
      drawAntNest(g, x, y, night);
    }
  for (const web of state.webs ?? []) {
    const { x, y } = plot(web.x, web.y);
    g.save();
    g.translate(x, y - 38);
    g.strokeStyle = night ? "#cfdfed9c" : "#fff8e5b8";
    g.lineWidth = 1;
    for (let i = 0; i < 8; i++) {
      const angle = i * Math.PI / 4;
      g.beginPath();
      g.moveTo(0, 0);
      g.lineTo(Math.cos(angle) * 28, Math.sin(angle) * 28);
      g.stroke();
    }
    for (const radius of [9, 18, 27]) {
      g.beginPath();
      for (let i = 0; i <= 8; i++) {
        const angle = i * Math.PI / 4;
        const px = Math.cos(angle) * radius, py = Math.sin(angle) * radius;
        if (!i)
          g.moveTo(px, py);
        else
          g.lineTo(px, py);
      }
      g.stroke();
    }
    g.restore();
  }
  for (const food of state.food ?? []) {
    const { x, y } = plot(food.x, food.y);
    g.fillStyle = "#e4b66c";
    for (let i = 0; i < Math.min(6, food.portions); i++) {
      const fx = x - 8 + i % 3 * 7, fy = y + Math.floor(i / 3) * 6;
      if (food.kind === "sunflower_seed") {
        g.fillStyle = "#645345";
        g.beginPath();
        g.ellipse(fx, fy, 2.5, 4, -0.4, 0, Math.PI * 2);
        g.fill();
        g.strokeStyle = "#dfd1ad";
        g.lineWidth = 1;
        g.beginPath();
        g.moveTo(fx - 1, fy - 2);
        g.lineTo(fx + 1, fy + 2);
        g.stroke();
      } else {
        g.fillStyle = "#e4b66c";
        g.beginPath();
        g.arc(fx, fy, 2.5, 0, Math.PI * 2);
        g.fill();
      }
    }
  }
  for (const [index, egg] of (state.eggs ?? []).filter((egg2) => !egg2.inside).entries()) {
    const { x, y } = plot(egg.x, egg.y);
    g.fillStyle = egg.species === "ant" ? "#eee2bd" : "#dbd9b0";
    g.beginPath();
    g.ellipse(x - 12 + index % 4 * 7, y - 4, 3, 4, -0.3, 0, Math.PI * 2);
    g.fill();
  }
  for (const plant of [...state.plants].sort((a, b) => a.y - b.y || a.x - b.x)) {
    const { x, y: base } = plot(plant.x, plant.y);
    g.save();
    g.translate(x, base);
    g.scale(scale, scale);
    g.translate(-x, -base);
    const appearance = plantAppearance(plant, night);
    if (appearance.wilt > 0) {
      g.translate(x, base);
      g.rotate(appearance.wilt * 0.16);
      g.scale(1, 1 - appearance.wilt * 0.12);
      g.translate(-x, -base);
    }
    if (drawNeighborPlant(g, plant, x, base, night)) {
      g.restore();
      continue;
    }
    const height = 13 + plant.stage * 20;
    const leaves = 1 + plant.stage * 2;
    g.strokeStyle = appearance.stem;
    g.lineWidth = 4 + plant.stage;
    g.lineCap = "round";
    g.beginPath();
    g.moveTo(x, base);
    g.quadraticCurveTo(x - 8, base - height * 0.6, x, base - height);
    g.stroke();
    for (let i = 0; i < leaves; i++) {
      const side = i % 2 ? -1 : 1;
      const y = base - height * (0.35 + 0.6 * i / Math.max(1, leaves - 1));
      drawLeaf(g, appearance, x + side * (9 + plant.stage * 3), y - 7 + appearance.wilt * 6, 11 + plant.stage * 3, plant.species === "fern" ? 7 : 11, -side * (0.45 - appearance.wilt * 0.8), i);
    }
    if (plant.species === "clover" && plant.stage > 1) {
      for (const [i, dx] of [-8, 0, 8].entries())
        drawLeaf(g, appearance, x + dx, base - height, 9, 9, 0, i);
    }
    drawPlantFlowers(g, plant, x, base - height, night);
    g.restore();
  }
  for (const facility of state.facilities ?? []) {
    const { x, y } = plot(facility.x, facility.y);
    g.save();
    g.translate(x, y);
    g.scale(scale, scale);
    g.translate(-x, -y);
    g.globalAlpha = facility.enabled ? 1 : 0.35;
    if (facility.kind === "mulch") {
      g.fillStyle = "#b79157";
      g.beginPath();
      g.ellipse(x, y, 24, 10, 0, 0, Math.PI * 2);
      g.fill();
      g.strokeStyle = "#74522f";
      g.lineWidth = 2;
      for (let i = -2; i <= 2; i++) {
        g.beginPath();
        g.moveTo(x + i * 7 - 3, y - 4);
        g.lineTo(x + i * 7 + 3, y + 4);
        g.stroke();
      }
    } else if (facility.kind === "shade") {
      g.strokeStyle = "#9b7958";
      g.lineWidth = 3;
      for (const dx of [-25, 25]) {
        g.beginPath();
        g.moveTo(x + dx, y + 4);
        g.lineTo(x + dx, y - 54);
        g.stroke();
      }
      g.fillStyle = "#638b76bb";
      g.beginPath();
      g.moveTo(x - 33, y - 54);
      g.lineTo(x + 28, y - 58);
      g.lineTo(x + 34, y - 41);
      g.lineTo(x - 28, y - 37);
      g.closePath();
      g.fill();
    } else if (facility.kind === "grow_light") {
      g.strokeStyle = "#68847b";
      g.lineWidth = 3;
      g.beginPath();
      g.moveTo(x + 18, y);
      g.lineTo(x + 18, y - 64);
      g.lineTo(x - 10, y - 64);
      g.stroke();
      g.fillStyle = night ? "#a49b70" : "#ffe9a0";
      g.beginPath();
      g.ellipse(x - 10, y - 60, 14, 5, 0, 0, Math.PI * 2);
      g.fill();
      if (!night && facility.enabled) {
        g.fillStyle = "#ffeda126";
        g.beginPath();
        g.moveTo(x - 22, y - 56);
        g.lineTo(x + 14, y);
        g.lineTo(x - 35, y);
        g.closePath();
        g.fill();
      }
    }
    g.restore();
  }
  for (const item of state.items) {
    const { x, y } = plot(item.x, item.y);
    g.fillStyle = "rgba(30,28,23,.25)";
    g.beginPath();
    g.ellipse(x, y + 7, 21, 6, 0, 0, Math.PI * 2);
    g.fill();
    g.fillStyle = item.id === "coin" ? "#d0a34d" : "#919a94";
    g.beginPath();
    g.ellipse(x, y, item.id === "coin" ? 16 : 19, 10, -0.1, 0, Math.PI * 2);
    g.fill();
    if (item.id === "coin") {
      g.strokeStyle = "#f4d682";
      g.lineWidth = 2;
      g.stroke();
    }
  }
  for (const seed of state.seeds ?? []) {
    const { x, y } = plot(seed.x, seed.y);
    g.fillStyle = "#d9bd75";
    g.beginPath();
    g.ellipse(x, y, 7, 4, -0.3, 0, Math.PI * 2);
    g.fill();
  }
  const surfaceBugs = state.bugs.filter((bug) => !bug.inside);
  const locations = bugPositions(surfaceBugs, positions, sceneSize(state));
  for (const bug of surfaceBugs) {
    const location = locations.get(bug.id);
    const host = bug.species === "aphid" || bug.species === "ladybug" ? state.plants.find((p) => p.x === bug.x && p.y === bug.y) : void 0;
    const onLeaf = host ? (13 + host.stage * 20) * 0.6 * scale : 0;
    const x = location.x, y = location.y - (bug.flying ? 30 * scale : onLeaf);
    g.save();
    g.translate(x, y);
    g.scale(scale, scale);
    g.translate(-x, -y);
    g.fillStyle = "rgba(33,28,20,.26)";
    g.beginPath();
    g.ellipse(x, y + 9, 14, 4, 0, 0, Math.PI * 2);
    g.fill();
    if (bug.flying) {
      g.fillStyle = bug.species === "moth" ? "#d9bfa1" : "rgba(228,246,247,.85)";
      const span = bug.species === "moth" ? 12 : bug.species === "mosquito" ? 8 : 7;
      for (const side of [-1, 1]) {
        g.beginPath();
        g.ellipse(x + side * span * 0.6, y - 3, span, span * 0.55, side * -0.55, 0, Math.PI * 2);
        g.fill();
      }
      g.fillStyle = bug.species === "fruit_fly" ? "#be8056" : bug.species === "moth" ? "#8a6851" : "#465357";
      g.beginPath();
      g.ellipse(x, y, 3, bug.species === "mosquito" ? 8 : 6, 0, 0, Math.PI * 2);
      g.fill();
      g.strokeStyle = "#5d5149";
      g.lineWidth = 1;
      for (const side of [-1, 1]) {
        g.beginPath();
        g.moveTo(x, y - 5);
        g.lineTo(x + side * 4, y - 11);
        g.stroke();
      }
      if (bug.species === "moth") {
        g.fillStyle = "#97745c";
        for (const side of [-1, 1]) {
          g.beginPath();
          g.arc(x + side * 9, y - 5, 2.4, 0, Math.PI * 2);
          g.fill();
        }
      }
    } else if (bug.species === "spider") {
      g.strokeStyle = "#282a36";
      g.lineWidth = 2.4;
      for (const side of [-1, 1])
        for (let i = -1; i <= 1; i++) {
          g.beginPath();
          g.moveTo(x + side * 5, y + i * 3);
          g.lineTo(x + side * 16, y + i * 8);
          g.stroke();
        }
      g.fillStyle = "#393848";
      g.beginPath();
      g.arc(x, y, 9, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = "#e3ca9d";
      for (const dx of [-3, 3]) {
        g.beginPath();
        g.arc(x + dx, y - 3, 1.5, 0, Math.PI * 2);
        g.fill();
      }
    } else if (bug.species === "ant") {
      g.strokeStyle = "#3b2b24";
      g.lineWidth = 1.8;
      for (const side of [-1, 1])
        for (const dx of [-5, 0, 5]) {
          g.beginPath();
          g.moveTo(x + dx, y);
          g.lineTo(x + dx - 2, y + side * 8);
          g.stroke();
        }
      g.fillStyle = "#493025";
      for (const [dx, radius] of [[-7, 5], [0, 3], [6, 4]]) {
        g.beginPath();
        g.ellipse(x + dx, y, radius, radius * 0.75, 0, 0, Math.PI * 2);
        g.fill();
      }
      for (const side of [-1, 1]) {
        g.beginPath();
        g.moveTo(x + 8, y + side * 2);
        g.lineTo(x + 14, y + side * 6);
        g.stroke();
      }
      if (bug.carrying) {
        g.fillStyle = "#edc06f";
        g.beginPath();
        g.arc(x + 13, y, 3, 0, Math.PI * 2);
        g.fill();
      }
    } else if (bug.species === "ladybug") {
      drawLadybug(g, x, y, night);
    } else if (bug.species === "beetle") {
      g.fillStyle = "#684337";
      g.beginPath();
      g.ellipse(x, y, 13, 9, 0, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = "#b57242";
      g.beginPath();
      g.ellipse(x, y - 2, 11, 7, 0, 0, Math.PI * 2);
      g.fill();
      g.strokeStyle = "#58372f";
      g.lineWidth = 1.6;
      g.beginPath();
      g.moveTo(x, y - 9);
      g.lineTo(x, y + 5);
      g.stroke();
      g.fillStyle = "#2c2a26";
      g.beginPath();
      g.arc(x + 10, y + 1, 2, 0, Math.PI * 2);
      g.fill();
    } else {
      g.fillStyle = "#9bc96c";
      g.beginPath();
      g.ellipse(x, y, 10, 7, 0, 0, Math.PI * 2);
      g.fill();
      g.strokeStyle = "#608943";
      g.lineWidth = 1.4;
      for (const side of [-1, 1])
        for (const dx of [-4, 1, 5]) {
          g.beginPath();
          g.moveTo(x + dx, y + side * 4);
          g.lineTo(x + dx + 3, y + side * 10);
          g.stroke();
        }
      g.fillStyle = "#24352b";
      g.beginPath();
      g.arc(x + 5, y - 2, 1.5, 0, Math.PI * 2);
      g.fill();
    }
    g.restore();
  }
  for (const hit of hits) {
    const { x, y } = plot(hit.x, hit.y);
    g.strokeStyle = `rgba(255,210,91,${hit.strength})`;
    g.lineWidth = 3;
    for (let i = 0; i < 6; i++) {
      const angle = i * Math.PI / 3;
      g.beginPath();
      g.moveTo(x + Math.cos(angle) * 8, y + Math.sin(angle) * 8);
      g.lineTo(x + Math.cos(angle) * (13 + 12 * (1 - hit.strength)), y + Math.sin(angle) * (13 + 12 * (1 - hit.strength)));
      g.stroke();
    }
  }
  g.strokeStyle = "rgba(255,255,255,.4)";
  g.lineWidth = 12;
  g.lineCap = "round";
  g.beginPath();
  g.moveTo(76, 100);
  g.lineTo(76, 173);
  g.moveTo(94, 92);
  g.lineTo(94, 140);
  g.stroke();
  g.strokeStyle = night ? "#a7c1c7" : "#eff8ea";
  g.lineWidth = 9;
  g.beginPath();
  g.roundRect(60, 70, 680, 400, 24);
  g.stroke();
  g.fillStyle = night ? "#40555a" : "#72906f";
  g.beginPath();
  g.roundRect(90, 460, 620, 15, 7);
  g.fill();
}
function createLandRenderer(canvas) {
  let state = null;
  let previous = /* @__PURE__ */ new Map();
  let movedAt = 0;
  let hitAt = 0;
  let hits = [];
  let frame = 0;
  let disposed = false;
  const currentPositions = (now) => {
    return new Map((state?.bugs ?? []).map((bug) => {
      const old = previous.get(bug.id) ?? bug;
      const t = Math.min(1, (now - movedAt) / (bug.species === "ant" ? 350 : 900));
      return [bug.id, { x: old.x + (bug.x - old.x) * t, y: old.y + (bug.y - old.y) * t }];
    }));
  };
  const render = (now) => {
    if (disposed || !state)
      return;
    drawScene(canvas, state, currentPositions(now), hits.map((hit) => ({ ...hit, strength: Math.max(0, 1 - (now - hitAt) / 900) })));
    frame = requestAnimationFrame(render);
  };
  return {
    setState(next) {
      if (disposed)
        return;
      const now = performance.now();
      const removed = state ? state.bugs.filter((bug) => !next.bugs.some((current) => current.id === bug.id) && next.bugs.some((hunter) => hunter.x === bug.x && hunter.y === bug.y && (hunter.species === "spider" ? bug.species !== "spider" : ["beetle", "ladybug"].includes(hunter.species) && bug.species === "aphid"))).map((bug) => ({ x: bug.x, y: bug.y })) : [];
      if (removed.length) {
        hits = removed;
        hitAt = now;
      }
      const oldPositions = currentPositions(now);
      const moved = next.bugs.some((bug) => {
        const old = state?.bugs.find((current) => current.id === bug.id);
        return old && (old.x !== bug.x || old.y !== bug.y);
      });
      if (moved) {
        previous = oldPositions;
        movedAt = now;
      }
      state = next;
      if (!frame)
        frame = requestAnimationFrame(render);
    },
    dispose() {
      disposed = true;
      cancelAnimationFrame(frame);
    }
  };
}
function createScene(canvas, language = "zh") {
  const renderer = createLandRenderer(canvas);
  const viewer = createNestViewer(canvas, language);
  return {
    setState(state, recordedAtMs) {
      renderer.setState(state);
      viewer.setState(state, recordedAtMs);
    },
    dispose() {
      viewer.dispose();
      renderer.dispose();
    }
  };
}

// ../../scratch/terrarium-0805/renderer-source/dist/water-scene.js
function createWaterScene(canvas) {
  let state;
  let old = /* @__PURE__ */ new Map();
  let movedAt = 0;
  let request = 0;
  let disposed = false;
  const position = (x, y) => cellPosition(WATER_SURFACE, x, y, sceneSize(state ?? {}));
  const locations = (now) => new Map((state?.fish ?? []).map((fish) => {
    const previous = old.get(fish.id) ?? fish;
    const t = Math.min(1, (now - movedAt) / 900);
    return [fish.id, { x: previous.x + (fish.x - previous.x) * t, y: previous.y + (fish.y - previous.y) * t }];
  }));
  const render = (now) => {
    if (disposed || !state)
      return;
    const g = canvas.getContext("2d");
    if (g) {
      const night = state.phase === "night";
      const size = sceneSize(state), scale = residentScale(size);
      g.fillStyle = night ? "#102e4b" : "#cbe9f1";
      g.fillRect(0, 0, 800, 500);
      const water = g.createLinearGradient(0, 70, 0, 470);
      water.addColorStop(0, night ? "#174762" : "#71c6db");
      water.addColorStop(1, night ? "#153540" : "#4193ac");
      g.fillStyle = water;
      g.beginPath();
      g.roundRect(60, 70, 680, 400, 20);
      g.fill();
      g.strokeStyle = "#d6f3ed";
      g.lineWidth = 7;
      g.stroke();
      g.fillStyle = "#b8a789";
      g.fillRect(66, 435, 668, 27);
      g.font = "16px system-ui";
      g.fillStyle = night ? "#def5fa" : "#164257";
      g.fillText(`O\u2082 ${state.oxygen.toFixed(1)} \xB7 ${state.cleanliness.toFixed(1)} / 100`, 72, 42);
      g.lineWidth = 1;
      g.strokeStyle = "#ffffff25";
      for (let x = 0; x < sceneSize(state); x++)
        for (let y = 0; y < sceneSize(state); y++) {
          const p = position(x, y);
          g.strokeRect(p.x - WATER_SURFACE.width / size / 2, p.y - WATER_SURFACE.height / size / 2, WATER_SURFACE.width / size, WATER_SURFACE.height / size);
        }
      for (const plant of state.plants) {
        const p = position(plant.x, plant.y), h = 22 + plant.stage * 9;
        g.save();
        g.translate(p.x, p.y);
        g.scale(scale, scale);
        g.translate(-p.x, -p.y);
        const appearance = plantAppearance(plant, night);
        g.strokeStyle = appearance.stem;
        g.lineWidth = 5;
        for (let strand = -1; strand <= 1; strand++) {
          g.beginPath();
          g.moveTo(p.x + strand * 10, p.y + 22);
          g.quadraticCurveTo(p.x + strand * 18 + Math.sin(now / 1e3) * 2, p.y, p.x + strand * 12, p.y + 22 - h * (1 - appearance.wilt * 0.35));
          g.stroke();
          if (plant.species !== "algae")
            for (let i = 0; i < 2; i++)
              drawLeaf(g, appearance, p.x + strand * 12 + (i % 2 ? -5 : 5), p.y + 12 - h * (0.25 + i * 0.28), 9, 4, (i % 2 ? -0.5 : 0.5) + appearance.wilt * 0.5, i);
        }
        if (plant.species === "waterlily") {
          g.fillStyle = (plant.health ?? 100) > 35 ? "#e7b8cf" : appearance.leaf;
          g.beginPath();
          g.ellipse(p.x, p.y - h + 22, 13, 6, 0, 0, Math.PI * 2);
          g.fill();
        }
        g.restore();
      }
      for (const food of state.food) {
        const p = position(food.x, food.y);
        g.fillStyle = "#f4ca77";
        for (let i = 0; i < food.portions; i++) {
          g.beginPath();
          g.arc(p.x + i % 4 * 5 - 8, p.y + Math.floor(i / 4) * 5, 2, 0, Math.PI * 2);
          g.fill();
        }
      }
      for (const facility of state.facilities ?? []) {
        const p = position(facility.x, facility.y);
        g.save();
        g.translate(p.x, p.y);
        g.scale(scale, scale);
        g.translate(-p.x, -p.y);
        g.globalAlpha = facility.enabled ? 1 : 0.35;
        if (facility.kind === "air_stone") {
          g.fillStyle = "#769cad";
          g.beginPath();
          g.ellipse(p.x, p.y + 12, 15, 7, 0, 0, Math.PI * 2);
          g.fill();
          if (facility.enabled)
            for (let i = 0; i < 5; i++) {
              const rise = (now / 35 + i * 18) % 90;
              g.strokeStyle = "#d4f8ff99";
              g.lineWidth = 1.5;
              g.beginPath();
              g.arc(p.x + Math.sin(i + now / 1200) * 9, p.y - rise, 2 + i % 3, 0, Math.PI * 2);
              g.stroke();
            }
        } else if (facility.kind === "filter") {
          g.fillStyle = "#547e82";
          g.beginPath();
          g.roundRect(p.x - 12, p.y - 35, 24, 46, 6);
          g.fill();
          g.strokeStyle = "#b4e9db";
          g.lineWidth = 2;
          for (let i = 0; i < 3; i++) {
            g.beginPath();
            g.moveTo(p.x - 7, p.y - 24 + i * 8);
            g.lineTo(p.x + 7, p.y - 24 + i * 8);
            g.stroke();
          }
        } else if (facility.kind === "water_light") {
          g.fillStyle = night ? "#7d8580" : "#f9e6a4";
          g.beginPath();
          g.roundRect(p.x - 23, p.y - 45, 46, 8, 4);
          g.fill();
          if (!night && facility.enabled) {
            g.fillStyle = "#ffffba25";
            g.beginPath();
            g.moveTo(p.x - 18, p.y - 37);
            g.lineTo(p.x + 30, p.y + 20);
            g.lineTo(p.x - 30, p.y + 20);
            g.closePath();
            g.fill();
          }
        }
        g.restore();
      }
      const points = locations(now);
      for (const fish of state.fish) {
        const p = position(points.get(fish.id).x, points.get(fish.id).y);
        const companions = state.fish.filter((other) => other.x === fish.x && other.y === fish.y).map((other) => other.id).sort();
        const index = companions.indexOf(fish.id), columns = Math.min(3, companions.length);
        const rows = Math.ceil(companions.length / columns);
        p.x += (index % columns - (Math.min(columns, companions.length - Math.floor(index / columns) * columns) - 1) / 2) * 24 * scale;
        p.y += (Math.floor(index / columns) - (rows - 1) / 2) * Math.min(16 * scale, 24 / rows);
        g.save();
        g.translate(p.x, p.y);
        g.scale(scale, scale);
        g.translate(-p.x, -p.y);
        g.fillStyle = fish.species === "goldfish" ? "#f3a545" : fish.species === "perch" ? "#718f78" : "#b9b3e5";
        g.beginPath();
        g.moveTo(p.x - 15, p.y);
        g.lineTo(p.x - 30, p.y - 12);
        g.lineTo(p.x - 30, p.y + 12);
        g.closePath();
        g.fill();
        g.beginPath();
        g.ellipse(p.x, p.y, fish.species === "perch" ? 24 : 18, 11, 0, 0, Math.PI * 2);
        g.fill();
        g.fillStyle = "#172e38";
        g.beginPath();
        g.arc(p.x + 10, p.y - 3, 2, 0, Math.PI * 2);
        g.fill();
        g.restore();
      }
    }
    request = requestAnimationFrame(render);
  };
  return {
    setState(next) {
      if (disposed)
        return;
      const now = performance.now();
      const current = locations(now);
      if (next.fish.some((fish) => {
        const previous = state?.fish.find((other) => other.id === fish.id);
        return previous && (previous.x !== fish.x || previous.y !== fish.y);
      })) {
        old = current;
        movedAt = now;
      }
      state = next;
      if (!request)
        request = requestAnimationFrame(render);
    },
    dispose() {
      disposed = true;
      cancelAnimationFrame(request);
    }
  };
}

// webui/playback.ts
function createPlaybackControls(container, createRenderer, language = "zh") {
  const copy = language === "zh" ? { title: "\u6355\u98DF\u56DE\u653E", empty: "\u6682\u65E0\u6355\u98DF\u8BB0\u5F55", play: "\u64AD\u653E", pause: "\u6682\u505C", frames: ["\u79FB\u52A8\u524D", "\u63A5\u89E6\u730E\u7269", "\u6355\u98DF\u540E"], select: "\u9009\u62E9\u6355\u98DF\u8BB0\u5F55", frame: "\u56DE\u653E\u5E27" } : { title: "Hunt replays", empty: "No recorded hunts", play: "Play", pause: "Pause", frames: ["Before movement", "Contact", "After capture"], select: "Choose a hunt", frame: "Replay frame" };
  const details = document.createElement("details");
  const summary = document.createElement("summary");
  summary.textContent = copy.title;
  const select = document.createElement("select");
  select.setAttribute("aria-label", copy.select);
  const play = document.createElement("button");
  play.type = "button";
  play.textContent = copy.play;
  const slider = document.createElement("input");
  slider.type = "range";
  slider.min = "0";
  slider.max = "2";
  slider.step = "1";
  slider.value = "0";
  slider.setAttribute("aria-label", copy.frame);
  const label = document.createElement("p");
  const canvas = document.createElement("canvas");
  canvas.width = 800;
  canvas.height = 500;
  canvas.style.width = "100%";
  canvas.style.maxWidth = "800px";
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", copy.title);
  const scene = createRenderer(canvas);
  details.append(summary, select, play, slider, label, canvas);
  container.append(details);
  let replays = [];
  let timer;
  let disposed = false;
  const stop = () => {
    clearTimeout(timer);
    timer = void 0;
    play.textContent = copy.play;
  };
  const render = () => {
    const replay = replays.find((replay2) => replay2.id === select.value);
    canvas.hidden = !replay;
    select.disabled = play.disabled = slider.disabled = !replay;
    if (!replay) {
      label.textContent = copy.empty;
      return;
    }
    const index = Number(slider.value);
    label.textContent = `${new Date(replay.recordedAtMs).toLocaleString(language)} \xB7 ${replay.hunterId} \u2192 ${replay.preyId} \xB7 ${copy.frames[index]}`;
    slider.setAttribute("aria-valuetext", copy.frames[index]);
    scene.setState(replay.frames[index], replay.recordedAtMs);
  };
  const advance = () => {
    if (disposed) return;
    slider.value = String(Number(slider.value) + 1);
    render();
    if (Number(slider.value) < 2) timer = setTimeout(advance, 1500);
    else stop();
  };
  play.addEventListener("click", () => {
    if (timer !== void 0) {
      stop();
      return;
    }
    slider.value = "0";
    render();
    play.textContent = copy.pause;
    timer = setTimeout(advance, 1500);
  });
  slider.addEventListener("input", () => {
    stop();
    render();
  });
  select.addEventListener("change", () => {
    stop();
    slider.value = "0";
    render();
  });
  render();
  return {
    setReplays(next) {
      if (disposed) return;
      const selected = select.value;
      const retained = next.some((replay) => replay.id === selected);
      replays = next;
      select.replaceChildren(...[...next].reverse().map((replay) => {
        const option = document.createElement("option");
        option.value = replay.id;
        option.textContent = `${new Date(replay.recordedAtMs).toLocaleString(language)} \xB7 ${replay.hunterId} \u2192 ${replay.preyId}`;
        return option;
      }));
      if (retained) select.value = selected;
      else {
        stop();
        slider.value = "0";
        render();
      }
    },
    dispose() {
      disposed = true;
      stop();
      scene.dispose();
      details.remove();
    }
  };
}

// webui/water-scene.ts
function createWaterScene2(canvas) {
  const renderer = createWaterScene(canvas);
  return { setState(state) {
    renderer.setState({ phase: state.phase, gridSize: capacity(state).size, facilities: state.facilities, oxygen: state.oxygen, cleanliness: state.cleanliness, plants: state.plants, fish: state.fish, food: state.food });
  }, dispose() {
    renderer.dispose();
  } };
}
function createWaterReplayControls(container, language = "zh") {
  return createPlaybackControls(container, createWaterScene2, language);
}

// webui/public-controls.ts
var nextId = 0;
function node(tag, className, text) {
  const element = document.createElement(tag);
  element.className = className;
  if (text !== void 0) element.textContent = text;
  return element;
}
function installStyles() {
  if (document.getElementById("terrarium-public-controls")) return;
  const style = node("style", "");
  style.id = "terrarium-public-controls";
  style.textContent = `
.pc-select{position:relative;min-width:0;font-size:12px;color:inherit}.pc-trigger{display:flex;align-items:center;justify-content:space-between;gap:16px;width:100%;border:1px solid #dde3d7;background:#fafbf5;color:inherit;border-radius:9px;padding:10px 12px;text-align:left;cursor:pointer;transition:background .2s,border-color .2s,box-shadow .2s}.pc-trigger>span:first-child{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.pc-chevron{flex:none;width:7px;height:7px;border-right:1.5px solid currentColor;border-bottom:1.5px solid currentColor;transform:rotate(45deg) translateY(-2px);transition:transform .22s}.pc-trigger[aria-expanded=true]{border-color:#9fb590;box-shadow:0 0 0 3px #91aa7d14}.pc-trigger[aria-expanded=true] .pc-chevron{transform:rotate(225deg) translate(-2px,-1px)}
.pc-menu{position:absolute;z-index:10;top:calc(100% + 7px);right:0;min-width:100%;width:max-content;max-width:min(440px,calc(100vw - 64px));max-height:240px;overflow:auto;padding:5px;border:1px solid #dde3d7;border-radius:11px;background:#fffefa;box-shadow:0 12px 32px #253f351c;opacity:0;visibility:hidden;transform:translateY(-6px) scale(.98);transform-origin:top right;transition:opacity .18s,transform .18s,visibility 0s .18s}.pc-select[data-open=true] .pc-menu{transition-delay:0s;opacity:1;visibility:visible;transform:translateY(0) scale(1)}.pc-option{display:flex;width:100%;align-items:center;justify-content:space-between;gap:14px;border:0;border-radius:6px;background:transparent;padding:10px 11px;text-align:left;color:inherit;cursor:pointer;transition:background .16s,color .16s}.pc-option>span:first-child{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.pc-option:hover,.pc-option:focus-visible{background:#edf1e5;outline:none}.pc-option[aria-selected=true]{background:#e7eddc;color:#355a45}.pc-tick{width:12px;height:6px;flex:none;border-left:1.5px solid currentColor;border-bottom:1.5px solid currentColor;transform:rotate(-45deg) scale(.4);opacity:0;transition:opacity .18s,transform .18s}.pc-option[aria-selected=true] .pc-tick{opacity:1;transform:rotate(-45deg) scale(1)}.pc-trigger:disabled,.pc-play:disabled{opacity:.45;cursor:default}
.pc-disclosure{border:0;background:transparent;color:#5d7253;display:flex;align-items:center;justify-content:space-between;gap:12px;width:100%;padding:16px 0;cursor:pointer;font-size:12px;text-align:left}.pc-disclosure .pc-chevron{transform:rotate(-45deg)}.pc-disclosure[aria-expanded=true] .pc-chevron{transform:rotate(45deg)}.pc-panel{display:grid;grid-template-rows:0fr;opacity:0;transition:grid-template-rows .32s cubic-bezier(.2,.7,.3,1),opacity .22s}.pc-panel[data-open=true]{grid-template-rows:1fr;opacity:1}.pc-panel-inner{min-height:0;overflow:hidden}.pc-replay-body{padding:0 0 18px}.pc-replay .pc-select{margin:0 0 16px}.pc-transport{display:flex;align-items:center;gap:18px;margin:16px 0}.pc-play{width:42px;height:38px;flex:none;display:grid;place-items:center;border:1px solid #d9e2ce;border-radius:10px;background:#e8eddf;color:#355a45;cursor:pointer;transition:transform .16s,background .16s}.pc-play:hover{background:#dae5cc}.pc-play:active{transform:scale(.94)}.pc-play-icon{width:0;height:0;border-left:10px solid currentColor;border-top:6px solid transparent;border-bottom:6px solid transparent;transition:transform .18s}.pc-play[aria-pressed=true] .pc-play-icon{width:10px;height:12px;border:0;border-left:3px solid currentColor;border-right:3px solid currentColor;transform:scale(.9)}
.pc-slider{position:relative;flex:1;min-width:60px;height:32px;touch-action:none;cursor:pointer;border-radius:8px}.pc-track{position:absolute;top:14px;left:8px;right:8px;height:4px;border-radius:8px;background:#e1e7d8}.pc-fill{height:100%;width:var(--progress,0%);border-radius:inherit;background:#7d9668;transition:width .22s ease}.pc-thumb{position:absolute;top:9px;left:calc(8px + (100% - 16px)*var(--fraction,0));width:14px;height:14px;margin-left:-7px;border:3px solid #fffefa;border-radius:50%;background:#64854f;box-shadow:0 0 0 1px #8aa474,0 2px 5px #355a4520;transition:left .22s ease,transform .16s,box-shadow .16s}.pc-slider:hover .pc-thumb,.pc-slider[data-dragging=true] .pc-thumb{transform:scale(1.2);box-shadow:0 0 0 4px #8aa47422}.pc-slider[aria-disabled=true]{opacity:.4;cursor:default}.pc-steps{display:flex;justify-content:space-between;gap:8px;margin:0 0 10px}.pc-step{font-size:10px!important;padding:4px 0!important;border:0!important;background:transparent!important;color:#7b8774;transition:color .2s}.pc-step[aria-pressed=true]{color:#355a45;font-weight:600}.pc-replay-label{font-size:11px;line-height:1.8;color:#748076;margin:10px 0}.pc-replay canvas{display:block;width:100%;border-radius:9px}.pc-select button,.pc-play,.pc-disclosure,.pc-step{font-family:inherit}.pc-slider:focus-visible,.pc-trigger:focus-visible,.pc-play:focus-visible,.pc-disclosure:focus-visible,.pc-step:focus-visible{outline:2px solid #88a77d;outline-offset:3px}
@media(prefers-reduced-motion:reduce){.pc-select *,.pc-panel,.pc-slider *,.pc-play,.pc-play-icon,.pc-disclosure *,.pc-step{transition:none!important}}
`;
  document.head.append(style);
}
function createPublicSelect(container, label, onChange) {
  installStyles();
  const abort = new AbortController(), signal = abort.signal;
  const root = node("div", "pc-select"), trigger = node("button", "pc-trigger"), caption = node("span", "", label), chevron = node("span", "pc-chevron"), menu = node("div", "pc-menu");
  trigger.type = "button";
  trigger.setAttribute("role", "combobox");
  trigger.setAttribute("aria-label", label);
  trigger.setAttribute("aria-haspopup", "listbox");
  menu.id = `pc-menu-${++nextId}`;
  menu.setAttribute("role", "listbox");
  menu.setAttribute("aria-label", label);
  trigger.setAttribute("aria-controls", menu.id);
  trigger.append(caption, chevron);
  root.append(trigger, menu);
  container.append(root);
  let options = [], value = "", open = false;
  const close = (focus = false) => {
    open = false;
    root.dataset.open = "false";
    trigger.setAttribute("aria-expanded", "false");
    menu.inert = true;
    if (focus) trigger.focus();
  };
  const sync = () => {
    caption.textContent = options.find((option) => option.value === value)?.label ?? label;
    Array.from(menu.children).forEach((option, index) => option.setAttribute("aria-selected", String(options[index].value === value)));
  };
  const focusOption = (index) => {
    const option = menu.children[(index + options.length) % options.length];
    option?.focus();
  };
  const show = () => {
    if (trigger.disabled || !options.length) return;
    open = true;
    root.dataset.open = "true";
    trigger.setAttribute("aria-expanded", "true");
    menu.inert = false;
    focusOption(Math.max(0, options.findIndex((option) => option.value === value)));
  };
  trigger.addEventListener("click", () => open ? close() : show());
  root.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      event.target.click();
    } else if (event.key === "Escape") {
      event.preventDefault();
      close(true);
    } else if (event.key === "Tab") close();
    else if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      if (!open) {
        show();
        return;
      }
      const index = Array.from(menu.children).indexOf(document.activeElement);
      focusOption(event.key === "Home" ? 0 : event.key === "End" ? options.length - 1 : index + (event.key === "ArrowDown" ? 1 : -1));
    } else if (open && event.key.length === 1 && event.key !== " ") {
      const index = options.findIndex((option) => option.label.toLocaleLowerCase().startsWith(event.key.toLocaleLowerCase()));
      if (index >= 0) {
        event.preventDefault();
        focusOption(index);
      }
    }
  });
  document.addEventListener("pointerdown", (event) => {
    if (!root.contains(event.target)) close();
  }, { signal });
  root.addEventListener("focusout", (event) => {
    if (!root.contains(event.relatedTarget)) close();
  });
  close();
  return {
    root,
    get value() {
      return value;
    },
    setOptions(next) {
      if (JSON.stringify(options) === JSON.stringify(next)) return;
      options = next;
      if (!options.some((option) => option.value === value)) value = options[0]?.value ?? "";
      menu.replaceChildren(...options.map((option) => {
        const button = node("button", "pc-option"), text = node("span", "", option.label), tick = node("span", "pc-tick");
        button.type = "button";
        button.tabIndex = -1;
        button.setAttribute("role", "option");
        button.append(text, tick);
        button.addEventListener("click", () => {
          const changed = value !== option.value;
          value = option.value;
          sync();
          close(true);
          if (changed) onChange(value);
        });
        return button;
      }));
      sync();
      if (!options.length) close();
    },
    setDisabled(disabled) {
      trigger.disabled = disabled;
      if (disabled) close();
    },
    dispose() {
      abort.abort();
      root.remove();
    }
  };
}
function createPublicReplayControls(container, createRenderer, language = "zh") {
  installStyles();
  const copy = language === "zh" ? { title: "\u6355\u98DF\u56DE\u653E", empty: "\u6682\u65E0\u6355\u98DF\u8BB0\u5F55", play: "\u64AD\u653E", pause: "\u6682\u505C", frames: ["\u79FB\u52A8\u524D", "\u63A5\u89E6\u730E\u7269", "\u6355\u98DF\u540E"], select: "\u9009\u62E9\u6355\u98DF\u8BB0\u5F55", frame: "\u56DE\u653E\u5E27" } : { title: "Hunt replays", empty: "No recorded hunts", play: "Play", pause: "Pause", frames: ["Before movement", "Contact", "After capture"], select: "Choose a hunt", frame: "Replay frame" };
  const root = node("div", "pc-replay"), toggle = node("button", "pc-disclosure", copy.title), chevron = node("span", "pc-chevron"), panel = node("div", "pc-panel"), inner = node("div", "pc-panel-inner"), body = node("div", "pc-replay-body"), selectHost = node("div", ""), transport = node("div", "pc-transport"), play = node("button", "pc-play"), icon = node("span", "pc-play-icon"), slider = node("div", "pc-slider"), track = node("div", "pc-track"), fill = node("div", "pc-fill"), thumb = node("div", "pc-thumb"), steps = node("div", "pc-steps"), label = node("p", "pc-replay-label"), canvas = node("canvas", "");
  toggle.type = play.type = "button";
  toggle.setAttribute("aria-expanded", "false");
  toggle.append(chevron);
  panel.id = `pc-panel-${++nextId}`;
  toggle.setAttribute("aria-controls", panel.id);
  panel.inert = true;
  play.append(icon);
  play.setAttribute("aria-label", copy.play);
  play.setAttribute("aria-pressed", "false");
  slider.tabIndex = 0;
  slider.setAttribute("role", "slider");
  slider.setAttribute("aria-label", copy.frame);
  slider.setAttribute("aria-valuemin", "0");
  slider.setAttribute("aria-valuemax", "2");
  track.append(fill);
  slider.append(track, thumb);
  canvas.width = 800;
  canvas.height = 500;
  canvas.setAttribute("role", "img");
  canvas.setAttribute("aria-label", copy.title);
  transport.append(play, slider);
  body.append(selectHost, transport, steps, label, canvas);
  inner.append(body);
  panel.append(inner);
  root.append(toggle, panel);
  container.append(root);
  let scene;
  let active = true;
  let replays = [], index = 0, timer, disposed = false, expanded = false;
  const stop = () => {
    clearTimeout(timer);
    timer = void 0;
    play.setAttribute("aria-label", copy.play);
    play.setAttribute("aria-pressed", "false");
  };
  const select = createPublicSelect(selectHost, copy.select, () => {
    stop();
    index = 0;
    render();
  });
  const stepButtons = copy.frames.map((text, i) => {
    const button = node("button", "pc-step", text);
    button.type = "button";
    button.addEventListener("click", () => {
      stop();
      index = i;
      render();
    });
    steps.append(button);
    return button;
  });
  const render = () => {
    const replay = replays.find((replay2) => replay2.id === select.value);
    canvas.hidden = transport.hidden = steps.hidden = !replay;
    select.setDisabled(!replay);
    play.disabled = !replay;
    slider.setAttribute("aria-disabled", String(!replay));
    slider.tabIndex = replay ? 0 : -1;
    slider.setAttribute("aria-valuenow", String(index));
    slider.setAttribute("aria-valuetext", copy.frames[index]);
    slider.style.setProperty("--fraction", String(index / 2));
    slider.style.setProperty("--progress", `${index * 50}%`);
    stepButtons.forEach((button, i) => button.setAttribute("aria-pressed", String(index === i)));
    if (!replay || !expanded || !active) {
      scene?.dispose();
      scene = void 0;
    }
    if (!replay) {
      label.textContent = copy.empty;
      return;
    }
    label.textContent = `${new Date(replay.recordedAtMs).toLocaleString(language)} \xB7 ${replay.hunterId} \u2192 ${replay.preyId} \xB7 ${copy.frames[index]}`;
    if (expanded && active) {
      scene ??= createRenderer(canvas);
      scene.setState(replay.frames[index], replay.recordedAtMs);
    }
  };
  toggle.addEventListener("click", () => {
    expanded = !expanded;
    toggle.setAttribute("aria-expanded", String(expanded));
    panel.dataset.open = String(expanded);
    panel.inert = !expanded;
    if (!expanded) stop();
    render();
  });
  const advance = () => {
    if (disposed) return;
    index++;
    render();
    if (index < 2) timer = setTimeout(advance, 1500);
    else stop();
  };
  play.addEventListener("click", () => {
    if (timer !== void 0) {
      stop();
      return;
    }
    index = 0;
    render();
    play.setAttribute("aria-label", copy.pause);
    play.setAttribute("aria-pressed", "true");
    timer = setTimeout(advance, 1500);
  });
  const seek = (clientX) => {
    if (!replays.length) return;
    const rect = slider.getBoundingClientRect();
    stop();
    index = Math.max(0, Math.min(2, Math.round((clientX - rect.left - 8) / (rect.width - 16) * 2)));
    render();
  };
  slider.addEventListener("pointerdown", (event) => {
    if (event.button !== 0 || !replays.length) return;
    event.preventDefault();
    slider.focus();
    slider.setPointerCapture(event.pointerId);
    slider.dataset.dragging = "true";
    seek(event.clientX);
  });
  slider.addEventListener("pointermove", (event) => {
    if (slider.hasPointerCapture(event.pointerId)) seek(event.clientX);
  });
  const release = (event) => {
    slider.dataset.dragging = "false";
    if (slider.hasPointerCapture(event.pointerId)) slider.releasePointerCapture(event.pointerId);
  };
  slider.addEventListener("pointerup", release);
  slider.addEventListener("pointercancel", release);
  slider.addEventListener("keydown", (event) => {
    if (!replays.length || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    stop();
    index = event.key === "Home" ? 0 : event.key === "End" ? 2 : Math.max(0, Math.min(2, index + (["ArrowRight", "ArrowUp"].includes(event.key) ? 1 : -1)));
    render();
  });
  render();
  return {
    setReplays(next) {
      if (disposed) return;
      const retained = next.some((replay) => replay.id === select.value);
      replays = next;
      select.setOptions([...next].reverse().map((replay) => ({ value: replay.id, label: `${new Date(replay.recordedAtMs).toLocaleString(language)} \xB7 ${replay.hunterId} \u2192 ${replay.preyId}` })));
      if (!retained) {
        stop();
        index = 0;
      }
      render();
    },
    setActive(value) {
      active = value;
      if (!active) stop();
      render();
    },
    dispose() {
      disposed = true;
      stop();
      select.dispose();
      scene?.dispose();
      root.remove();
    }
  };
}

// webui/scene.ts
function landScene(state) {
  return { phase: state.phase, gridSize: capacity(state).size, facilities: state.facilities, eggs: state.eggs, nests: state.nests?.map((n) => ({ ...n, foodCapacity: n.depth * 20 })), food: state.food, webs: state.webs, seeds: state.seeds, items: state.items, soil: state.soil?.map((cell) => ({ ...cell, daylight: daylight(state, cell.x, cell.y) })), plants: state.plants.map((p) => ({ ...p, flower: flowerStage(p), seedHead: hasSeedHead(p) })), bugs: state.bugs.map((b) => ({ ...b, flying: isFlying(b.species) })) };
}
function drawScene2(canvas, state, positions, hits) {
  drawScene(canvas, landScene(state), positions, hits);
}
function createLandRenderer2(canvas) {
  const renderer = createLandRenderer(canvas);
  return { setState(state) {
    renderer.setState(landScene(state));
  }, dispose() {
    renderer.dispose();
  } };
}
function createScene2(canvas, language = "zh") {
  const renderer = createScene(canvas, language);
  return { setState(state, recordedAtMs) {
    renderer.setState(landScene(state), recordedAtMs);
  }, dispose() {
    renderer.dispose();
  } };
}
function createReplayControls(container, language = "zh") {
  return createPlaybackControls(container, (canvas) => createScene2(canvas, language), language);
}
export {
  bugPositions,
  createLandRenderer2 as createLandRenderer,
  createPublicReplayControls,
  createPublicSelect,
  createReplayControls,
  createScene2 as createScene,
  createWaterReplayControls,
  createWaterScene2 as createWaterScene,
  drawScene2 as drawScene,
  landScene,
  plotPosition
};
