export const TERRARIUM_MAX_DEATH_RECORDS = 48;
export interface DeathRecord {
  recordedAtMs: number;
  organism: { id: string; species: string; x: number; y: number; energy?: number; hunger?: number; health?: number; ageHours?: number; lastMeal?: { atMs: number; kind: string; id?: string } };
  cause: string;
  environment: Record<string, number | string>;
}
export interface DeathHistory { deaths?: DeathRecord[] }

/** Records the final observed state before removal; manual removals are excluded. */
export function recordDeath(state: DeathHistory, organism: DeathRecord['organism'], recordedAtMs: number, cause: string, environment: DeathRecord['environment']): void {
  (state.deaths ??= []).push({ recordedAtMs, organism: structuredClone(organism), cause, environment: { ...environment } });
  state.deaths = state.deaths.slice(-TERRARIUM_MAX_DEATH_RECORDS);
}
export function validDeaths(value: unknown, atMs: number, gridSize = 5): boolean {
  if (value === undefined) return true;
  return Array.isArray(value) && value.length <= TERRARIUM_MAX_DEATH_RECORDS && value.every((r, i) => {
    if (!r || !Number.isFinite(r.recordedAtMs) || r.recordedAtMs > atMs || (i > 0 && r.recordedAtMs < value[i - 1].recordedAtMs) || typeof r.cause !== 'string' || r.cause.length > 1000) return false;
    const o = r.organism;
    return o && typeof o.id === 'string' && o.id.length <= 32 && typeof o.species === 'string' && o.species.length <= 32 && [o.x, o.y].every(n => Number.isInteger(n) && n >= 0 && n < gridSize) && ['energy', 'hunger', 'health', 'ageHours'].every(k => o[k] === undefined || Number.isFinite(o[k]) && o[k] >= 0) && validMeal(o.lastMeal, r.recordedAtMs) && r.environment && typeof r.environment === 'object' && !Array.isArray(r.environment) && Object.keys(r.environment).length <= 10 && Object.values(r.environment).every(v => typeof v === 'number' ? Number.isFinite(v) : typeof v === 'string' && v.length <= 100);
  });
}
export function validMeal(value: unknown, atMs: number): boolean {
  if (value === undefined) return true;
  if (!value || typeof value !== 'object') return false;
  const meal = value as { atMs?: number; kind?: string; id?: string };
  return Number.isFinite(meal.atMs) && meal.atMs! <= atMs && typeof meal.kind === 'string' && meal.kind.length <= 32 && (meal.id === undefined || typeof meal.id === 'string' && meal.id.length <= 32);
}
