import type { AquariumState } from './aquarium.ts';
import type { TerrariumState } from './state.ts';

export const TERRARIUM_SCORE_VERSION = 2;
export type ScoreParts = { plants: number; bugs: number; seeds: number } | { plants: number; fish: number; water: number };

/** Only the three strongest members of each species score, at 100%, 50% and 25%. */
function speciesScore<T extends { species: string }>(organisms: readonly T[], value: (organism: T) => number): number {
  const groups = new Map<string, number[]>();
  for (const organism of organisms) {
    const values = groups.get(organism.species) ?? [];
    values.push(value(organism)); groups.set(organism.species, values);
  }
  return Math.round([...groups.values()].reduce((sum, values) => sum + values.sort((a, b) => b - a).slice(0, 3).reduce((total, v, i) => total + v * [1, .5, .25][i], 0), 0));
}
const maturity = (ageHours: number): number => .25 + .75 * Math.min(1, ageHours / 24);
export function scoreParts(state: TerrariumState | AquariumState): ScoreParts {
  const plants = speciesScore<{ species: string; stage: number; health: number }>(state.plants, plant => (plant.stage + 1) * plant.health / 10);
  if ('fish' in state) {
    const fish = speciesScore(state.fish, fish => 40 * fish.health / 100 * fish.energy / 100 * maturity(fish.ageHours) * Math.min(1, state.oxygen / 50, state.cleanliness / 50));
    return { plants, fish, water: Math.round(40 * Math.min(state.oxygen, state.cleanliness) / 100 * Math.min(1, plants / 40, fish / 40)) };
  }
  return {
    plants,
    bugs: speciesScore(state.bugs, bug => 40 * bug.energy / 100 * (1 - (bug.hunger ?? 0) / 100) * maturity(bug.ageHours)),
    seeds: 0,
  };
}
export function score(state: TerrariumState | AquariumState): number {
  return Object.values(scoreParts(state)).reduce((sum, part) => sum + part, 0);
}
export interface LeaderboardBoxSummary { rank: number; competitiveRank?: number; id: string; name?: string; score: number; scoreParts: ScoreParts; reportedAt: string }
export interface LeaderboardSummary { habitat: 'land' | 'water'; total: number; entries: LeaderboardBoxSummary[]; own: LeaderboardBoxSummary | null }

export const TERRARIUM_SCORE_RULES = '同种只取贡献最高的前三个，权重 100%/50%/25%；种子和额外浇水不直接计分。动物成熟度在 24 个模拟小时后达到上限，饥饿或健康、能量与水环境影响动物分。水环境分 = round(40 × min(氧气, 水质)/100 × min(1, 植物分/40, 鱼分/40))。';
