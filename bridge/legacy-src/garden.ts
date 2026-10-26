import type { TerrariumState, PlantState, BugState } from './state.ts';
import type { AquariumState } from './aquarium.ts';
import { inHabitat, capacity, hasFacility } from './progression.ts';
import { isNestEntrance } from './colony.ts';
import { TERRARIUM_PLANT_REQUIREMENTS } from './habitat.ts';
import { TERRARIUM_MAX_COINS } from './merchant.ts';

export const TERRARIUM_CROPS = {
  tomato: { name: '番茄', firstHours: 18, intervalHours: 8, portions: 3, price: 3 },
  strawberry: { name: '草莓', firstHours: 12, intervalHours: 6, portions: 2, price: 4 },
} as const;
export type CropSpecies = keyof typeof TERRARIUM_CROPS;
export interface GardenState {
  neighborCursor?: number;
  seedPackets?: Record<string, number>;
  produce?: Partial<Record<CropSpecies, number>>;
  litter?: Array<{ x: number; y: number; portions: number }>;
  decomposition?: number;
  decorations?: Array<{ x: number; y: number; kind: 'stone_path' | 'wood_path' | 'pebbles' }>;
  antPolicy?: { breedingEnabled: boolean; protectPlantsWhenFed: boolean; maxPopulation: number };
}
export function cropRounds(plant: PlantState): number {
  const crop = TERRARIUM_CROPS[plant.species as CropSpecies];
  return crop ? Math.max(0, Math.floor(((plant.fruitGrowthHours ?? 0) - crop.firstHours) / crop.intervalHours) + 1) : 0;
}
export function gardenStatus(state: TerrariumState) {
  return { produce: state.produce ?? {}, crops: state.plants.filter(p => Object.hasOwn(TERRARIUM_CROPS, p.species)).map(p => ({ id: p.id, species: p.species, readyRounds: Math.min(3, Math.max(0, cropRounds(p) - (p.harvestRounds ?? 0))), naturalHours: p.fruitGrowthHours ?? 0 })), litter: state.litter ?? [], decorations: state.decorations ?? [], antPolicy: state.antPolicy ?? { breedingEnabled: true, protectPlantsWhenFed: true, maxPopulation: capacity(state).organisms } };
}
export function harvest(state: TerrariumState, id: unknown) {
  const plant = state.plants.find(p => p.id === id);
  if (!plant || !Object.hasOwn(TERRARIUM_CROPS, plant.species)) throw new Error('Select a tomato or strawberry plant');
  const species = plant.species as CropSpecies, crop = TERRARIUM_CROPS[species];
  const rounds = Math.min(3, Math.max(0, cropRounds(plant) - (plant.harvestRounds ?? 0)));
  if (!rounds || plant.health < 40) throw new Error('No ripe healthy fruit; fruit needs natural growth, fertilizer does not ripen it');
  const bonus = plant.pollination ?? 0;
  const portions = rounds * crop.portions + Math.min(rounds, bonus);
  if ((state.produce?.[species] ?? 0) + portions > 10000) throw new Error('Fruit basket is full');
  (state.produce ??= {})[species] = (state.produce?.[species] ?? 0) + portions;
  plant.harvestRounds = cropRounds(plant); plant.pollination = Math.max(0, bonus - rounds);
  return { id: plant.id, species, portions, basket: state.produce };
}
export function sellProduce(state: TerrariumState, species: unknown, portions: unknown) {
  if (typeof species !== 'string' || !Object.hasOwn(TERRARIUM_CROPS, species) || !Number.isSafeInteger(portions) || Number(portions) < 1) throw new Error('Supply crop species and positive integer portions');
  const crop = species as CropSpecies, amount = Number(portions);
  if ((state.produce?.[crop] ?? 0) < amount) throw new Error('Not enough harvested fruit');
  const coins = amount * TERRARIUM_CROPS[crop].price;
  if ((state.economy?.earned ?? 0) + coins > TERRARIUM_MAX_COINS) throw new Error('Coin earnings limit reached');
  state.economy ??= { earned: 0, spent: 0, inventory: {}, pots: [] };
  state.economy.earned += coins; state.produce![crop]! -= amount;
  return { species, portions: amount, coins, balance: state.economy.earned - state.economy.spent };
}
export function transplant(state: TerrariumState | AquariumState, id: unknown, x: number, y: number) {
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  const plants = 'fish' in state ? state.plants : [...state.plants, ...state.seeds];
  const plant = plants.find(p => p.id === id);
  if (!plant) throw new Error('Unknown living plant or seed');
  if (plants.some(p => p !== plant && p.x === x && p.y === y) || !('fish' in state) && isNestEntrance(state, x, y)) throw new Error('Target plot is occupied');
  plant.x = x; plant.y = y;
  if (!('fish' in state) && 'moisture' in plant) {
    plant.moisture = state.soil[y * capacity(state).size + x].moisture;
    plant.dry = plant.moisture < TERRARIUM_PLANT_REQUIREMENTS[plant.species].minMoisture;
  }
  return { plant, potsStayOnTheirPlots: true };
}
export function addLitter(state: TerrariumState, x: number, y: number, portions = 1) {
  const litter = state.litter ??= [], old = litter.find(p => p.x === x && p.y === y);
  if (old) old.portions = Math.min(20, old.portions + portions);
  else if (litter.length < 64) litter.push({ x, y, portions: Math.min(20, portions) });
}
export function actDecomposer(state: TerrariumState, bug: BugState) {
  const litter = [...(state.litter ?? [])].sort((a, b) => Math.abs(a.x - bug.x) + Math.abs(a.y - bug.y) - Math.abs(b.x - bug.x) - Math.abs(b.y - bug.y))[0];
  if (!litter) return;
  if (bug.x !== litter.x) bug.x += Math.sign(litter.x - bug.x);
  else if (bug.y !== litter.y) bug.y += Math.sign(litter.y - bug.y);
  else if (bug.energy < 85 || (bug.hunger ?? 0) >= 25) {
    litter.portions--; bug.energy = Math.min(100, bug.energy + 20); bug.hunger = Math.max(0, (bug.hunger ?? 0) - 40);
    if (!litter.portions) state.litter!.splice(state.litter!.indexOf(litter), 1);
    state.decomposition = (state.decomposition ?? 0) + 1;
    if (hasFacility(state, 'compost_bin') && state.decomposition >= 3) { state.decomposition -= 3; state.fertilizer = (state.fertilizer ?? 0) + 1; }
    state.decomposition = Math.min(3, state.decomposition);
  }
}
export function validGarden(state: TerrariumState): boolean {
  const count = (n: unknown, max: number) => Number.isSafeInteger(n) && Number(n) >= 0 && Number(n) <= max;
  const unique = (values: Array<{ x: number; y: number }>) => new Set(values.map(p => `${p?.x},${p?.y}`)).size === values.length;
  return (state.neighborCursor === undefined || count(state.neighborCursor, Number.MAX_SAFE_INTEGER)) && (state.seedPackets === undefined || !!state.seedPackets && typeof state.seedPackets === 'object' && !Array.isArray(state.seedPackets) && Object.entries(state.seedPackets).every(([k,v]) => Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS,k) && count(v,10000))) && (state.produce === undefined || !!state.produce && typeof state.produce === 'object' && !Array.isArray(state.produce) && Object.entries(state.produce).every(([k, v]) => Object.hasOwn(TERRARIUM_CROPS, k) && count(v, 10000))) &&
    (state.litter === undefined || Array.isArray(state.litter) && state.litter.length <= 64 && unique(state.litter) && state.litter.every(p => p && inHabitat(state, p.x, p.y) && count(p.portions, 20) && p.portions > 0)) &&
    (state.decomposition === undefined || count(state.decomposition, 3)) &&
    (state.decorations === undefined || Array.isArray(state.decorations) && state.decorations.length <= 64 && unique(state.decorations) && state.decorations.every(p => p && inHabitat(state, p.x, p.y) && ['stone_path', 'wood_path', 'pebbles'].includes(p.kind))) &&
    (state.antPolicy === undefined || !!state.antPolicy && typeof state.antPolicy.breedingEnabled === 'boolean' && typeof state.antPolicy.protectPlantsWhenFed === 'boolean' && count(state.antPolicy.maxPopulation, 32)) &&
    state.plants.every(p => p && (p.fruitGrowthHours === undefined || Object.hasOwn(TERRARIUM_CROPS, p.species) && Number.isFinite(p.fruitGrowthHours) && p.fruitGrowthHours >= 0 && p.fruitGrowthHours <= p.growthHours + 1e-8) && (p.harvestRounds === undefined || Number.isSafeInteger(p.harvestRounds) && p.harvestRounds >= 0 && p.harvestRounds <= cropRounds(p)));
}
