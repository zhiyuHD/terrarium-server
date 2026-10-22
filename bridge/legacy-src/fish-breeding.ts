import type { AquariumState, FishState } from './aquarium.ts';
import { capacity } from './progression.ts';

export const TERRARIUM_FISH_BREEDING = { maturityHours: 24, juvenileHours: 12, intervalHours: 12, incubationHours: 4, eggsPerClutch: 2 } as const;
export const TERRARIUM_SPAWNING_SPECIES = ['neon_tetra', 'corydoras'] as const;
export interface FishEgg {
  id: string; species: typeof TERRARIUM_SPAWNING_SPECIES[number]; x: number; y: number;
  laidAtMs: number; incubationHours: number; health: number;
}
const HOUR_MS = 3_600_000;

export function fishLifeStage(fish: Pick<FishState, 'juvenile'>): 'juvenile' | 'adult' { return fish.juvenile ? 'juvenile' : 'adult'; }
export function fishPopulation(state: Pick<AquariumState, 'fish' | 'eggs'>): number { return state.fish.length + (state.eggs?.length ?? 0); }
export function clutchCapacity(population: number, limit: number, maximum: number): number {
  if (population / limit >= .9) return 0;
  return Math.max(0, Math.min(limit - population, population / limit >= .75 ? 1 : maximum));
}

/** Eggs reserve fish capacity; suitable water advances incubation, while poor water or lost cover damages eggs. */
export function advanceFishBreeding(state: AquariumState, startMs: number, endMs: number): { laid: number; hatched: number; lost: number } {
  const result = { laid: 0, hatched: 0, lost: 0 };
  const cover = state.plants.filter(p => p.stage >= 2 && p.health >= 60);
  for (const fish of state.fish) if (fish.juvenile && fish.ageHours >= TERRARIUM_FISH_BREEDING.juvenileHours) delete fish.juvenile;
  if (Math.floor(startMs / HOUR_MS) !== Math.floor(endMs / HOUR_MS) && state.oxygen >= 60 && state.cleanliness >= 60 && cover.length) {
    const eligible = (fish: FishState) => (TERRARIUM_SPAWNING_SPECIES as readonly string[]).includes(fish.species) && !fish.juvenile &&
      fish.ageHours >= TERRARIUM_FISH_BREEDING.maturityHours && fish.ageHours - (fish.lastSpawnAgeHours ?? 0) >= TERRARIUM_FISH_BREEDING.intervalHours && fish.health >= 80 && fish.energy >= 70;
    for (const fish of state.fish) {
      const count = clutchCapacity(fishPopulation(state), capacity(state).organisms, TERRARIUM_FISH_BREEDING.eggsPerClutch);
      if (!count) break;
      if (!eligible(fish)) continue;
      const mate = state.fish.find(other => other !== fish && other.species === fish.species && eligible(other) && Math.abs(other.x - fish.x) + Math.abs(other.y - fish.y) <= 1);
      const plant = cover.find(p => Math.abs(p.x - fish.x) + Math.abs(p.y - fish.y) <= 2);
      if (!mate || !plant) continue;
      for (const parent of [fish, mate]) { parent.lastSpawnAgeHours = parent.ageHours; parent.energy -= 15; }
      for (let i = 0; i < count; i++) (state.eggs ??= []).push({ id: `fish-egg-${state.nextId++}`, species: fish.species as FishEgg['species'], x: plant.x, y: plant.y, laidAtMs: endMs, incubationHours: 0, health: 100 });
      result.laid += count;
    }
  }
  for (const egg of [...(state.eggs ?? [])]) {
    const hours = Math.max(0, endMs - Math.max(startMs, egg.laidAtMs)) / HOUR_MS;
    const sheltered = cover.some(p => p.x === egg.x && p.y === egg.y);
    const good = state.oxygen >= 60 && state.cleanliness >= 60;
    const fair = state.oxygen >= 40 && state.cleanliness >= 40;
    egg.incubationHours = Math.min(TERRARIUM_FISH_BREEDING.incubationHours, egg.incubationHours + hours * (good ? 1 : fair ? .5 : 0));
    egg.health = Math.max(0, egg.health - hours * ((!fair ? 4 : 0) + (!sheltered ? 2 : 0)));
    if (!egg.health) { state.eggs!.splice(state.eggs!.indexOf(egg), 1); result.lost++; }
    else if (egg.incubationHours >= TERRARIUM_FISH_BREEDING.incubationHours - 1e-9) {
      state.eggs!.splice(state.eggs!.indexOf(egg), 1);
      state.fish.push({ id: `fish-${state.nextId++}`, species: egg.species, x: egg.x, y: egg.y, energy: 70, health: egg.health, ageHours: 0, juvenile: true });
      result.hatched++;
    }
  }
  return result;
}
