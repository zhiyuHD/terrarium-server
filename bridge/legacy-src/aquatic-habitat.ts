import type { AquaticPlantState } from './aquarium.ts';

export const TERRARIUM_FISH_SPECIES = ['guppy', 'goldfish', 'perch', 'neon_tetra', 'corydoras'] as const;
export type FishSpecies = typeof TERRARIUM_FISH_SPECIES[number];
export const TERRARIUM_AQUATIC_PLANTS = {
  waterweed: { dayOxygen: .8, nightOxygen: -.2, purification: 0 },
  waterlily: { dayOxygen: .5, nightOxygen: -.1, purification: 0 },
  algae: { dayOxygen: 1.1, nightOxygen: -.3, purification: 0 },
  hornwort: { dayOxygen: 1.2, nightOxygen: -.25, purification: 0 },
  duckweed: { dayOxygen: .6, nightOxygen: -.15, purification: .12 },
} as const;
export type AquaticPlantSpecies = keyof typeof TERRARIUM_AQUATIC_PLANTS;
/** New plants provide their special benefits once mature and healthy. */
export function aquaticBenefits(plant: AquaticPlantState) { return plant.stage >= 2 && plant.health >= 60; }
export function aquaticFlower(plant: AquaticPlantState): 'bud' | 'bloom' | undefined {
  if (plant.species !== 'waterlily' || plant.health < 35) return undefined;
  return plant.growthHours >= 18 ? 'bloom' : plant.growthHours >= 12 ? 'bud' : undefined;
}
