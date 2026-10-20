import { capacity, nearFacility, type DevelopmentState } from './progression.ts';
export type PlantSpecies = 'fern' | 'clover' | 'moss' | 'basil' | 'bean' | 'sunflower' | 'mint';
export interface PlantRequirements { minMoisture: number; maxMoisture: number; minLight: number; maxLight: number }
export interface SoilCell { x: number; y: number; moisture: number; allowAntForaging?: boolean }
export const TERRARIUM_LEAF_DAMAGE = { perHarvestHealth: 5, recoveryPerGrowthHour: 2 } as const;
export const TERRARIUM_PLANT_LOW_HEALTH = 20;
export const TERRARIUM_PLANT_RECOVERED_HEALTH = 35;
export interface Habitat extends DevelopmentState { items: ReadonlyArray<{ id: string; x: number; y: number }> }

/** Simulation conditions, expressed as soil moisture (0–100) and daylight (0–1). */
export const TERRARIUM_PLANT_REQUIREMENTS: Record<PlantSpecies, PlantRequirements> = {
  fern: { minMoisture: 30, maxMoisture: 85, minLight: .15, maxLight: .7 },
  clover: { minMoisture: 20, maxMoisture: 80, minLight: .45, maxLight: 1 },
  moss: { minMoisture: 45, maxMoisture: 90, minLight: .15, maxLight: .4 },
  basil: { minMoisture: 30, maxMoisture: 75, minLight: .65, maxLight: 1 },
  bean: { minMoisture: 25, maxMoisture: 70, minLight: .7, maxLight: 1 },
  sunflower: { minMoisture: 25, maxMoisture: 70, minLight: .75, maxLight: 1 },
  mint: { minMoisture: 35, maxMoisture: 85, minLight: .3, maxLight: .85 },
};

export interface WateredPlant {
  id: string; species: PlantSpecies; x: number; y: number; moisture: number; health: number;
}

export function wateringReport(plants: ReadonlyArray<WateredPlant>, cells: ReadonlyArray<SoilCell>) {
  const watered = plants.filter(plant => cells.some(cell => cell.x === plant.x && cell.y === plant.y)).map(plant => {
    const { minMoisture, maxMoisture } = TERRARIUM_PLANT_REQUIREMENTS[plant.species];
    const moisture = cells.find(cell => cell.x === plant.x && cell.y === plant.y)!.moisture;
    return { ...plant, moisture, minMoisture, maxMoisture, belowMaximum: maxMoisture - moisture, moistureDeficit: Math.max(0, minMoisture - moisture), moistureStatus: moisture > maxMoisture ? 'high' as const : moisture < minMoisture ? 'low' as const : 'suitable' as const };
  });
  const warnings = watered.filter(plant => plant.moistureStatus !== 'suitable').map(plant =>
    `${plant.id}（${plant.species}）(${plant.x},${plant.y}) 水分${plant.moistureStatus === 'high' ? '过高' : '过低'}：${plant.moisture.toFixed(1)}，适宜 ${plant.minMoisture}–${plant.maxMoisture}；持续超限每小时损失 0.5 健康。`);
  return { plants: watered, warnings };
}

export function daylight(habitat: Habitat, x: number, y: number): number {
  const shaded = habitat.items.some(item => item.id === 'pebble' && Math.abs(item.x - x) + Math.abs(item.y - y) <= 1);
  let light = (.6 + x * .4 / (capacity(habitat).size - 1)) * (shaded ? .35 : 1);
  if (nearFacility(habitat, 'shade', x, y)) light *= .6;
  if (nearFacility(habitat, 'grow_light', x, y)) light = Math.max(.75, light);
  return light;
}

export interface HistorySample {
  gridSize?: number;
  recordedAtMs: number;
  score?: number; scoreVersion?: number;
  soilMoisture: number[];
  plants: Array<{ id: string; species: PlantSpecies; moisture: number; health: number; stage: number; growthHours: number }>;
}
export function plantCareAdvice(plant: { species: PlantSpecies; moisture: number }): string {
  const needs = TERRARIUM_PLANT_REQUIREMENTS[plant.species];
  if (plant.moisture > needs.maxMoisture) return `先停止浇水，可用 terrarium_drain 排水至 ${needs.maxMoisture} 以下；当前超出上限 ${(plant.moisture - needs.maxMoisture).toFixed(1)}。`;
  if (plant.moisture < needs.minMoisture) return `建议用 terrarium_water 补至目标水分 ${(needs.minMoisture + needs.maxMoisture) / 2}，避免超过 ${needs.maxMoisture}。`;
  return '当前水分适宜，无需为回血反复浇水；检查取食损伤，必要时禁止该格蚂蚁采集。';
}
export const TERRARIUM_MAX_HISTORY = 168;

export const TERRARIUM_SUNFLOWER = { seedHours: 30, seedIntervalHours: 12, seedPortions: 3 } as const;
export function sunflowerSeedRounds(growthHours: number): number {
  return Math.max(0, Math.floor((growthHours - TERRARIUM_SUNFLOWER.seedHours) / TERRARIUM_SUNFLOWER.seedIntervalHours) + 1);
}
export function hasSeedHead(plant: { species: string; growthHours?: number }): boolean {
  return plant.species === 'sunflower' && sunflowerSeedRounds(plant.growthHours ?? 0) > 0;
}
export const TERRARIUM_FLOWER_BUD_HOURS = 12;
export const TERRARIUM_FLOWER_BLOOM_HOURS = 18;

/** Flowers follow accumulated suitable daylight growth; fern and moss do not flower. */
export function flowerStage(plant: { species: string; growthHours?: number; health: number; moisture: number; dry: boolean }): 'bud' | 'bloom' | undefined {
  if (plant.species !== 'clover' && plant.species !== 'basil' && plant.species !== 'bean' && plant.species !== 'sunflower' && plant.species !== 'mint') return;
  if (hasSeedHead(plant)) return;
  const needs = TERRARIUM_PLANT_REQUIREMENTS[plant.species];
  if (plant.health < TERRARIUM_PLANT_RECOVERED_HEALTH || plant.dry || plant.moisture < needs.minMoisture || plant.moisture > needs.maxMoisture) return;
  if ((plant.growthHours ?? 0) >= TERRARIUM_FLOWER_BLOOM_HOURS) return 'bloom';
  if ((plant.growthHours ?? 0) >= TERRARIUM_FLOWER_BUD_HOURS) return 'bud';
}

export interface WateringOptions { amount?: number; targetMoisture?: number; onlyDry?: boolean }
export function validateWatering(options: WateringOptions): void {
  if ((options.amount !== undefined && (typeof options.amount !== 'number' || !Number.isFinite(options.amount) || options.amount <= 0 || options.amount > 100)) ||
      (options.targetMoisture !== undefined && (typeof options.targetMoisture !== 'number' || !Number.isFinite(options.targetMoisture) || options.targetMoisture < 0 || options.targetMoisture > 100)) ||
      (options.amount !== undefined && options.targetMoisture !== undefined) || (options.onlyDry !== undefined && typeof options.onlyDry !== 'boolean')) throw new Error('Supply amount (0..100, excluding zero) OR targetMoisture (0..100), and optional onlyDry');
}
export function waterAdded(moisture: number, options: WateringOptions): number {
  validateWatering(options);
  return Math.max(0, Math.min(100 - moisture, options.targetMoisture === undefined ? options.amount ?? 35 : options.targetMoisture - moisture));
}

export const TERRARIUM_GROWTH_STAGE_HOURS = [0, 2, 6, 12] as const;
