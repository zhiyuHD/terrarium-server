import { capacity, inHabitat, recordAction, type Progression } from './progression.ts';
import { TERRARIUM_PLANT_REQUIREMENTS } from './habitat.ts';
import { newState, validState, type TerrariumState, type PlantState, type SeedState, type BugState } from './state.ts';
import { newAquarium, validAquarium, type AquariumState, type FishState, type AquaticPlantState } from './aquarium.ts';

export type CollectionItem = { kind: 'plant'; specimen: PlantState } | { kind: 'seed'; specimen: SeedState } | { kind: 'bug'; specimen: BugState } | { kind: 'fish'; specimen: FishState } | { kind: 'aquatic'; specimen: AquaticPlantState };
/** Collected organisms are frozen, excluded from simulation and scoring, and restored into normal capacity limits. */
export function collect(state: TerrariumState | AquariumState, id: string): CollectionItem {
  if ((state.collection?.length ?? 0) >= capacity(state).collection) throw new Error('Collection capacity reached');
  const groups = 'fish' in state ? [['fish', state.fish], ['aquatic', state.plants]] as const : [['plant', state.plants], ['seed', state.seeds], ['bug', state.bugs]] as const;
  for (const [kind, group] of groups) {
    const index = group.findIndex(p => p.id === id);
    if (index < 0) continue;
    const specimen = structuredClone(group[index]);
    if (kind === 'bug') { const bug = specimen as BugState; delete bug.inside; delete bug.role; delete bug.carrying; }
    const entry = { kind, specimen } as CollectionItem;
    group.splice(index, 1); (state.collection ??= []).push(entry);
    if (!('fish' in state) && kind === 'bug') state.webs = state.webs?.filter(web => web.spiderId !== id);
    recordAction(state, 'collect');
    return entry;
  }
  throw new Error('Unknown collectible organism');
}
export function restoreCollected(state: TerrariumState | AquariumState, id: string, x: number, y: number): void {
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  const entry = state.collection?.find(p => p.specimen.id === id);
  if (!entry) throw new Error('Unknown collected organism');
  const specimen = { ...structuredClone(entry.specimen), x, y };
  if ('fish' in state) {
    if (entry.kind === 'fish') {
      if (state.fish.length >= capacity(state).organisms) throw new Error('Fish capacity reached'); state.fish.push(specimen as FishState);
    } else if (entry.kind === 'aquatic') {
      if (state.plants.length >= capacity(state).organisms || state.plants.some(p => p.x === x && p.y === y)) throw new Error('Aquatic capacity or plot occupied: select a cell without aquatic plants; the specimen remains in collection'); state.plants.push(specimen as AquaticPlantState);
    } else throw new Error('Collection habitat mismatch');
  } else if (entry.kind === 'bug') {
    if (state.bugs.length + (state.eggs?.length ?? 0) >= capacity(state).organisms) throw new Error('Bug capacity reached'); state.bugs.push(specimen as BugState);
  } else if (entry.kind === 'plant' || entry.kind === 'seed') {
    if (state.plants.length + state.seeds.length >= capacity(state).organisms || [...state.plants, ...state.seeds].some(p => p.x === x && p.y === y)) throw new Error('Plant capacity or plot occupied: select a cell without a plant or seed; the specimen remains in collection');
    if (entry.kind === 'plant') { const plant = specimen as PlantState; plant.moisture = state.soil[y * capacity(state).size + x].moisture; plant.dry = plant.moisture < TERRARIUM_PLANT_REQUIREMENTS[plant.species].minMoisture; state.plants.push(plant); }
    else state.seeds.push(specimen as SeedState);
  } else throw new Error('Collection habitat mismatch');
  state.collection!.splice(state.collection!.indexOf(entry), 1);
}
export function validCollection(value: unknown, habitat: 'land' | 'water', limit = 48): boolean {
  if (value === undefined) return true;
  if (!Array.isArray(value) || value.length > limit || new Set(value.map(e => e?.specimen?.id)).size !== value.length) return false;
  const specimenProgress: Progression = { level: 4, peakScore: 420, guideStep: 0, guideSkipped: false, actions: [] };
  return value.every(entry => {
    if (!entry || !entry.specimen) return false;
    if (habitat === 'water') {
      const base = newAquarium(0); base.progression = specimenProgress; base.plants = []; base.fish = [];
      if (entry.kind === 'fish') { base.fish = [entry.specimen]; base.updatedAtMs = entry.specimen.lastMeal?.atMs ?? 0; } else if (entry.kind === 'aquatic') base.plants = [entry.specimen]; else return false;
      return validAquarium(base);
    }
    const base = newState(0); base.progression = specimenProgress; base.soil = Array.from({ length: 64 }, (_, i) => ({ x: i % 8, y: Math.floor(i / 8), moisture: 50 })); base.history = []; base.plants = []; base.bugs = []; base.seeds = [];
    if (entry.kind === 'plant') base.plants = [entry.specimen]; else if (entry.kind === 'seed') base.seeds = [entry.specimen]; else if (entry.kind === 'bug') { base.bugs = [entry.specimen]; base.updatedAtMs = entry.specimen.lastMeal?.atMs ?? 0; } else return false;
    return validState(base);
  });
}
