export const TERRARIUM_FLY_ACTION_INTERVAL_MS = 10_000;
export const TERRARIUM_FLY_MOVE_PROBABILITY = .75;
export const TERRARIUM_FLY_SPECIES = ['fruit_fly', 'mosquito', 'moth'] as const;
export type FlyingSpecies = typeof TERRARIUM_FLY_SPECIES[number];
export function isFlying(species: string): species is FlyingSpecies { return (TERRARIUM_FLY_SPECIES as readonly string[]).includes(species); }

/** An action's random draw is stable across saves, polling rates and offline catch-up. */
export function flightDraw(id: string, atMs: number): number {
  let value = 2166136261;
  for (const char of `${id}:${Math.floor(atMs / TERRARIUM_FLY_ACTION_INTERVAL_MS)}`) value = Math.imul(value ^ char.charCodeAt(0), 16777619);
  value = Math.imul(value ^ value >>> 16, 0x7feb352d);
  value = Math.imul(value ^ value >>> 15, 0x846ca68b);
  return ((value ^ value >>> 16) >>> 0) / 4294967296;
}
