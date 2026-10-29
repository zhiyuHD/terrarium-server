import type { AquariumState } from './aquarium.ts';
import type { TerrariumState } from './state.ts';

export const TERRARIUM_SCORE_VERSION = 3;
export type ScoreParts = { plants: number; bugs: number; seeds: number } | { plants: number; fish: number; water: number };

/** The curve has slope one at 200, then decreasing marginal returns without a ceiling. */
export function curvedScore(base: number): number {
  return Math.round(base <= 200 ? base : 200 + 200 * Math.log(base / 200));
}
export function recordedScore(version: number | undefined, value = 0): number {
  return version === 2 ? curvedScore(value) : version === TERRARIUM_SCORE_VERSION ? value : 0;
}
function curveParts(parts: ScoreParts): ScoreParts {
  const entries = Object.entries(parts), base = entries.reduce((sum, [, value]) => sum + value, 0);
  if (base <= 200) return parts;
  const total = curvedScore(base);
  const shares = entries.map(([key, value], index) => ({ key, index, exact: value * total / base, points: Math.floor(value * total / base) }));
  const remainder = total - shares.reduce((sum, share) => sum + share.points, 0);
  shares.sort((a, b) => (b.exact - b.points) - (a.exact - a.points) || a.index - b.index);
  for (const share of shares.slice(0, remainder)) share.points++;
  shares.sort((a, b) => a.index - b.index);
  return Object.fromEntries(shares.map(share => [share.key, share.points])) as ScoreParts;
}

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
    return curveParts({ plants, fish, water: Math.round(40 * Math.min(state.oxygen, state.cleanliness) / 100 * Math.min(1, plants / 40, fish / 40)) });
  }
  return curveParts({
    plants,
    bugs: speciesScore(state.bugs, bug => 40 * bug.energy / 100 * (1 - (bug.hunger ?? 0) / 100) * maturity(bug.ageHours)),
    seeds: 0,
  });
}
export function score(state: TerrariumState | AquariumState): number {
  return Object.values(scoreParts(state)).reduce((sum, part) => sum + part, 0);
}
export interface LeaderboardBoxSummary { rank: number; competitiveRank?: number; id: string; name?: string; score: number; scoreParts: ScoreParts; reportedAt: string }
export interface LeaderboardSummary { habitat: 'land' | 'water'; total: number; entries: LeaderboardBoxSummary[]; own: LeaderboardBoxSummary | null }

export const TERRARIUM_SCORE_RULES = '先计算基础分 B：同种只取贡献最高的前三个，权重 100%/50%/25%；植物贡献=(阶段+1)×健康/10；地箱动物贡献=40×能量/100×(1−饥饿/100)×成熟度，水箱动物贡献=40×健康/100×能量/100×成熟度×min(1,氧气/50,水质/50)；成熟度=0.25+0.75×min(1,年龄模拟小时/24)。各类基础分分别四舍五入，种子和额外浇水不直接计分。水环境基础分=round(40×min(氧气,水质)/100×min(1,植物基础分/40,鱼基础分/40))。总分：B≤200 时为 B，否则 round(200+200×ln(B/200))，ln 为自然对数，不设固定封顶；分项按基础贡献比例分配整数余数，合计等于总分。';
