import { flowerStage, type PlantSpecies } from './habitat.ts';
import type { BugState, PlantState } from './state.ts';

export const TERRARIUM_POLLINATION = { nectarIntervalMs: 30 * 60_000, maxCharges: 3, bonusSeedPortions: 2 } as const;
export function isPollinator(species: string): boolean { return species === 'bee' || species === 'butterfly'; }
export function producesSeeds(species: string): boolean { return ['sunflower', 'daisy', 'lavender'].includes(species); }
export interface Pollen { species: PlantSpecies; plantId: string }

/** Only a visit carrying pollen from a different plant of the same species fertilizes a flower. */
export function visitFlower(bug: BugState, plant: PlantState, atMs: number): boolean {
  if (!isPollinator(bug.species) || flowerStage(plant) !== 'bloom' || bug.x !== plant.x || bug.y !== plant.y ||
      bug.lastMeal && atMs - bug.lastMeal.atMs < TERRARIUM_POLLINATION.nectarIntervalMs) return false;
  const crossed = bug.pollen?.species === plant.species && bug.pollen.plantId !== plant.id;
  const fertilized = crossed && producesSeeds(plant.species) && (plant.pollination ?? 0) < TERRARIUM_POLLINATION.maxCharges;
  if (fertilized) plant.pollination = Math.min(TERRARIUM_POLLINATION.maxCharges, (plant.pollination ?? 0) + 1);
  bug.pollen = { species: plant.species, plantId: plant.id };
  bug.energy = Math.min(100, bug.energy + 20);
  bug.hunger = Math.max(0, (bug.hunger ?? 0) - 40);
  bug.lastMeal = { atMs, kind: 'foraging' };
  return fertilized;
}
