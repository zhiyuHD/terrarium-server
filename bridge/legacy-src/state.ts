import { TERRARIUM_LEVELS, capacity, inHabitat, hasFacility, observeProgress, recordAction, validDevelopment, type DevelopmentState } from './progression.ts';
import type { CareAlertState } from './care-alerts.ts';
import { validCareAlerts } from './care-alerts.ts';
import { recordDeath, validDeaths, validMeal, type DeathHistory } from './deaths.ts';
import { validCollection, type CollectionItem } from './collection.ts';
import { score, TERRARIUM_SCORE_VERSION } from './scoring.ts';
import { plantCareAdvice, TERRARIUM_LEAF_DAMAGE, sunflowerSeedRounds, TERRARIUM_SUNFLOWER } from './habitat.ts';
import { validateWatering, waterAdded, type WateringOptions } from './habitat.ts';
import { isFlying, flightDraw, TERRARIUM_FLY_ACTION_INTERVAL_MS, TERRARIUM_FLY_MOVE_PROBABILITY, type FlyingSpecies } from './flight.ts';
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import { ensureQueen, spreadAnts, surfaceAntOccupied, emptyEntrancePlot, isNestEntrance, nestEntrances, nestSize, digColony, fillAntEntrance, enterNest, walkInside, validColony, TERRARIUM_NEST_ENTRANCE, TERRARIUM_NEST_STORAGE, TERRARIUM_NEST_NURSERY, type NestPosition } from './colony.ts';
import { flowerStage, daylight, TERRARIUM_MAX_HISTORY, TERRARIUM_PLANT_LOW_HEALTH, TERRARIUM_PLANT_RECOVERED_HEALTH, TERRARIUM_PLANT_REQUIREMENTS, type HistorySample, type PlantSpecies, type SoilCell } from './habitat.ts';

import { captureFrame, TERRARIUM_MAX_REPLAYS, TERRARIUM_REPLAY_RETENTION_MS, type HuntReplay } from './replay.ts';

export type PlantId = string;
export type ItemId = 'coin' | 'pebble';
export type SeedSpecies = PlantSpecies;
export type BugSpecies = 'aphid' | 'beetle' | 'ladybug' | 'spider' | 'ant' | FlyingSpecies;
export interface PlantState { id: PlantId; species: PlantSpecies; x: number; y: number; moisture: number; growthHours: number; stage: number; dry: boolean; health: number; healthWarning?: boolean; seedRounds?: number; leafDamage?: number }
export interface SeedState { id: string; species: SeedSpecies; x: number; y: number; ageHours: number }
export interface BugState { id: string; species: BugSpecies; x: number; y: number; ageHours: number; energy: number; hunger?: number; lastMeal?: { atMs: number; kind: 'foraging' }; carrying?: number; role?: 'queen' | 'worker'; inside?: NestPosition }
export interface EggState { id: string; species: 'beetle' | 'ladybug' | 'spider' | 'ant' | FlyingSpecies; x: number; y: number; laidAtMs: number; hatchesAtMs: number; inside?: NestPosition }
export interface NestState { id: string; x: number; y: number; depth: number; food: number; size?: number; digProgress?: number; entrances?: Array<{ x: number; y: number }> }
export interface SpiderWeb { id: string; spiderId: string; x: number; y: number; createdAtMs: number }
export interface LandFood { id: string; x: number; y: number; portions: number; droppedAtMs: number; kind?: 'sunflower_seed' }
export interface ItemState { id: ItemId; x: number; y: number }
export interface TerrariumState extends DeathHistory, CareAlertState, DevelopmentState { collection?: CollectionItem[]; irrigation?: Array<{ x: number; y: number; targetMoisture: number; intervalMinutes: number; lastWateredAtMs: number }>; version: 3; name?: string; webs?: SpiderWeb[]; eggs?: EggState[]; nests?: NestState[]; food?: LandFood[]; replays?: HuntReplay[]; soil: SoilCell[]; history: HistorySample[]; updatedAtMs: number; nextId: number; plants: PlantState[]; seeds: SeedState[]; bugs: BugState[]; items: ItemState[] }
export interface Change { kind: 'stage' | 'dry' | 'health' | 'phase' | 'sprout' | 'hunt' | 'death' | 'birth' | 'hungry' | 'egg' | 'nest' | 'web' | 'irrigation'; text: string }

export const TERRARIUM_MAX_NAME_LENGTH = 64;

function validName(value: unknown): value is string {
  return typeof value === 'string' && value === value.trim() && Array.from(value).length >= 1 && Array.from(value).length <= TERRARIUM_MAX_NAME_LENGTH && !/[\u0000-\u001f\u007f-\u009f]/.test(value);
}

export function renameBox(state: Pick<TerrariumState, 'name'>, name: unknown): string {
  const trimmed = typeof name === 'string' ? name.trim() : name;
  if (!validName(trimmed)) throw new Error('Name must contain 1..64 characters without control characters');
  state.name = trimmed;
  return trimmed;
}

const STAGES = [0, 2, 6, 12];
export const TERRARIUM_BUG_ACTION_INTERVAL_MS = 60_000;
export const TERRARIUM_ANT_ACTION_INTERVAL_MS = 10_000;
export const TERRARIUM_PREDATOR_SENSE_RADIUS = 2;
export const TERRARIUM_PREDATOR_SATIATED_ENERGY = 80;
export const TERRARIUM_BUG_STARVATION_HOURS = 24;
export const TERRARIUM_BUG_HUNGER_WARNING = 80;
export const TERRARIUM_BUG_FORAGE_HUNGER = 25;
export const TERRARIUM_WEB_LIFETIME_MS = 6 * 60 * 60_000;
export const TERRARIUM_WEBS_PER_SPIDER = 3;
export const TERRARIUM_BREEDING = { maturityHours: 12, pairIntervalHours: 6, queenMaturityHours: 6, queenIntervalHours: 3, eggsPerClutch: 3, incubationHours: 1 } as const;
const HOUR_MS = 60 * 60_000;
export const TERRARIUM_MAX_CATCHUP_MS = 30 * 24 * HOUR_MS;

export function newState(nowMs: number): TerrariumState {
  const state: TerrariumState = { version: 3, soil: Array.from({ length: 25 }, (_, i) => ({ x: i % 5, y: Math.floor(i / 5), moisture: 50 })), history: [], updatedAtMs: nowMs, nextId: 1, plants: [
    { id: 'fern', species: 'fern', x: 0, y: 0, moisture: 45, growthHours: 0, stage: 0, dry: false, health: 100 },
    { id: 'clover', species: 'clover', x: 4, y: 0, moisture: 50, growthHours: 0, stage: 0, dry: false, health: 100 },
  ], seeds: [], bugs: [], items: [{ id: 'coin', x: 1, y: 1 }, { id: 'pebble', x: 2, y: 1 }] };
  for (const plant of state.plants) state.soil[plant.y * capacity(state).size + plant.x].moisture = plant.moisture;
  recordHistory(state, nowMs);
  return state;
}


export function sow(state: TerrariumState, species: SeedSpecies, x: number, y: number): SeedState {
  if (!Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS, species)) throw new Error('Unknown seed species');
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  if (state.plants.length + state.seeds.length >= capacity(state).organisms) throw new Error('Plant capacity reached');
  if (isNestEntrance(state, x, y) || [...state.plants, ...state.seeds].some(p => p.x === x && p.y === y)) throw new Error('That plot is occupied');
  const seed = { id: `seed-${state.nextId++}`, species, x, y, ageHours: 0 };
  state.seeds.push(seed); recordAction(state, 'plant'); return seed;
}

export function releaseBug(state: TerrariumState, species: BugSpecies, x: number, y: number): BugState {
  if (!['aphid', 'beetle', 'ladybug', 'spider', 'ant'].includes(species) && !isFlying(species)) throw new Error('Unknown bug species');
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  if (state.bugs.length + (state.eggs?.length ?? 0) >= capacity(state).organisms) throw new Error('Bug capacity reached');
  if (species === 'ant' && surfaceAntOccupied(state, x, y)) throw new Error('That surface cell already has an ant');
  const bug = { id: `bug-${state.nextId++}`, species, x, y, ageHours: 0, energy: 60, hunger: 0 };
  state.bugs.push(bug); recordAction(state, 'inhabit'); return bug;
}

export function feedBug(state: TerrariumState, id: string): BugState {
  const bug = state.bugs.find(value => value.id === id);
  if (!bug) throw new Error('Unknown bug');
  bug.energy = Math.min(100, bug.energy + 35);
  return bug;
}

export function relocateBug(state: TerrariumState, id: string, x: number, y: number): BugState {
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  const bug = state.bugs.find(value => value.id === id);
  if (!bug) throw new Error('Unknown bug');
  if (bug.species === 'ant' && surfaceAntOccupied(state, x, y, bug)) throw new Error('That surface cell already has an ant');
  bug.x = x; bug.y = y;
  delete bug.inside;
  return bug;
}

export { fillAntEntrance };

export function removeOrganism(state: TerrariumState, id: string): { id: string; species: string } {
  const nest = state.nests?.find(n => n.id === id);
  if (nest) { for (const p of nestEntrances(nest)) fillAntEntrance(state, p.x, p.y); return { id, species: 'nest' }; }
  const groups = [state.plants, state.seeds, state.bugs, state.eggs ?? [], state.food ?? [], state.webs ?? []];
  for (const group of groups) {
    const index = group.findIndex(value => value.id === id);
    if (index !== -1) {
      const [removed] = group.splice(index, 1);
      if ('species' in removed && removed.species === 'spider') state.webs = (state.webs ?? []).filter(web => web.spiderId !== removed.id);
      return { id: removed.id, species: 'species' in removed ? removed.species : 'spiderId' in removed ? 'web' : 'food' };
    }
  }
  throw new Error('Unknown organism');
}

export function dropLandFood(state: TerrariumState, x: number, y: number, portions: number): LandFood {
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  if (!Number.isInteger(portions) || portions < 1 || portions > 10) throw new Error('Food portions must be 1..10');
  if ((state.food?.length ?? 0) >= 20) throw new Error('Food capacity reached');
  const food = { id: `food-${state.nextId++}`, x, y, portions, droppedAtMs: state.updatedAtMs };
  (state.food ??= []).push(food);
  return food;
}

function approach(bug: BugState, target: { x: number; y: number }): boolean {
  if (bug.x !== target.x) bug.x += Math.sign(target.x - bug.x);
  else if (bug.y !== target.y) bug.y += Math.sign(target.y - bug.y);
  return bug.x === target.x && bug.y === target.y;
}
function nearest<T extends { id: string; x: number; y: number }>(bug: BugState, values: T[]): T | undefined {
  return [...values].sort((a, b) => Math.abs(a.x - bug.x) + Math.abs(a.y - bug.y) - Math.abs(b.x - bug.x) - Math.abs(b.y - bug.y) || a.id.localeCompare(b.id))[0];
}
type PlantDamage = (plant: PlantState, amount: number, cause: string) => void;
const label = (value: { id: string; species: string; x: number; y: number }): string => `${value.id}（${value.species}），位置 (${value.x},${value.y})`;

function actAnt(state: TerrariumState, bug: BugState, changes: Change[], damage: PlantDamage): void {
  const nests = state.nests ??= [];
  const entrances = nests.flatMap(n => nestEntrances(n).map(p => ({ ...p, id: n.id })));
  let nest = nests.find(n => n.id === (bug.inside?.nestId ?? nearest(bug, entrances)?.id));
  if (!nest) {
    const size = capacity(state).size;
    const plot = Array.from({ length: size * size }, (_, i) => ({ x: i % size, y: Math.floor(i / size) }))
      .filter(p => emptyEntrancePlot(state, p.x, p.y, bug)).sort((a, b) => Math.abs(a.x - bug.x) + Math.abs(a.y - bug.y) - Math.abs(b.x - bug.x) - Math.abs(b.y - bug.y) || a.y - b.y || a.x - b.x)[0];
    if (!plot || !approach(bug, plot)) return;
    nest = { id: `nest-${state.nextId++}`, x: bug.x, y: bug.y, depth: 1, food: 0 };
    nests.push(nest);
    changes.push({ kind: 'nest', text: `${label(bug)} 在 (${nest.x},${nest.y}) 挖出蚁巢 ${nest.id}。` });
    enterNest(bug, nest);
    return;
  }
  const entrance = [...nestEntrances(nest)].sort((a,b) => Math.abs(a.x-bug.x)+Math.abs(a.y-bug.y)-Math.abs(b.x-bug.x)-Math.abs(b.y-bug.y))[0];
  const availableFood = (state.food?.length ?? 0) > 0 || state.plants.some(p => p.health > 40 && state.soil[p.y * capacity(state).size + p.x].allowAntForaging !== false);
  const forageAlone = availableFood && bug.role === 'queen' && !state.bugs.some(other => other.species === 'ant' && other.role === 'worker') && nest.food <= 0 && (bug.energy < 70 || (bug.hunger ?? 0) >= TERRARIUM_BUG_FORAGE_HUNGER);
  if (bug.inside) {
    nest = nests.find(nest => nest.id === bug.inside!.nestId)!;
    if (bug.carrying) {
      if (walkInside(bug.inside, TERRARIUM_NEST_STORAGE)) { nest.food = Math.min(nest.depth * 20, nest.food + bug.carrying); bug.carrying = 0; }
      return;
    }
    if ((bug.energy < 70 || (bug.hunger ?? 0) >= TERRARIUM_BUG_FORAGE_HUNGER) && nest.food > 0) { nest.food--; bug.energy = Math.min(100, bug.energy + 20); bug.hunger = Math.max(0, (bug.hunger ?? 0) - 40); return; }
    if (bug.role === 'queen') {
      if (walkInside(bug.inside, forageAlone ? TERRARIUM_NEST_ENTRANCE : TERRARIUM_NEST_NURSERY) && forageAlone) { Object.assign(bug, entrance); delete bug.inside; }
      return;
    }
    if (nest.depth < 5) {
      nest.depth++;
      if (nest.depth === 5) changes.push({ kind: 'nest', text: `${nest.id} 挖至 5 层，可储存 100 份食物。` });
      return;
    }
    const dig = digColony(state, nest);
    if (dig.expanded) changes.push({ kind: 'nest', text: `${nest.id} 扩建为 ${nestSize(nest)}×${nestSize(nest)}，每格容纳一只蚂蚁。` });
    if (dig.entrance) changes.push({ kind: 'nest', text: `${nest.id} 在空地 (${dig.entrance.x},${dig.entrance.y}) 新建入口。` });
    if ((nest.digProgress ?? 0) > 0 || dig.expanded) return;
    if (walkInside(bug.inside, TERRARIUM_NEST_ENTRANCE)) { Object.assign(bug, entrance); delete bug.inside; }
    return;
  }
  if (bug.role === 'queen' && !forageAlone) { if (approach(bug, entrance)) enterNest(bug, nest); return; }
  if (bug.carrying) {
    if (approach(bug, entrance)) enterNest(bug, nest);
    return;
  }
  if ((bug.energy < 70 || (bug.hunger ?? 0) >= TERRARIUM_BUG_FORAGE_HUNGER) && nest.food > 0) {
    if (approach(bug, entrance)) enterNest(bug, nest);
    return;
  }
  if (bug.role !== 'queen' && bug.x === entrance.x && bug.y === entrance.y && nest.depth < 5) {
    enterNest(bug, nest);
    return;
  }
  if (nest.food >= Math.min(nest.depth * 20, Math.max(3, state.bugs.filter(b => b.species === 'ant').length * 2))) { if (approach(bug, entrance)) enterNest(bug, nest); return; }
  const food = nearest(bug, state.food ?? []);
  if (food) {
    if (approach(bug, food)) {
      food.portions--;
      if (forageAlone) { bug.energy = Math.min(100, bug.energy + 20); bug.hunger = Math.max(0, (bug.hunger ?? 0) - 40); }
      else bug.carrying = 1;
      if (!food.portions) state.food!.splice(state.food!.indexOf(food), 1);
    }
    return;
  }
  const plant = nearest(bug, state.plants.filter(plant => plant.health > 40 && state.soil[plant.y * capacity(state).size + plant.x].allowAntForaging !== false));
  if (plant && approach(bug, plant)) {
    damage(plant, 1, '蚂蚁采集');
    if (forageAlone) { bug.energy = Math.min(100, bug.energy + 20); bug.hunger = Math.max(0, (bug.hunger ?? 0) - 40); }
    else bug.carrying = 1;
  }
}

function actBug(state: TerrariumState, bug: BugState, changes: Change[], recordedAtMs: number, currentPhase: 'day' | 'night', damage: PlantDamage): void {
  const predator = bug.species !== 'aphid';
  const atEntrance = (position: { x: number; y: number }): boolean => isNestEntrance(state, position.x, position.y);
  const step = (dx: number, dy: number): boolean => {
    const next = { x: bug.x + dx, y: bug.y + dy };
    if ((!dx && !dy) || !inHabitat(state, next.x, next.y) || (predator && atEntrance(next))) return false;
    bug.x = next.x; bug.y = next.y; return true;
  };
  if (predator && atEntrance(bug)) {
    for (const [dx, dy] of [[1, 0], [0, 1], [-1, 0], [0, -1]]) if (step(dx, dy)) break;
    return;
  }
  if (predator && bug.energy >= TERRARIUM_PREDATOR_SATIATED_ENERGY && (bug.hunger ?? 0) < TERRARIUM_BUG_FORAGE_HUNGER) return;
  if (!predator && bug.energy >= 70 && (bug.hunger ?? 0) < TERRARIUM_BUG_FORAGE_HUNGER && state.plants.some(p => p.health > 0 && p.x === bug.x && p.y === bug.y)) return;
  const targets = bug.species === 'aphid' ? state.plants.filter(p => p.health > 0) : state.bugs.filter(other =>
    other.id !== bug.id && !other.inside && !isFlying(other.species) && !atEntrance(other) &&
    Math.abs(other.x - bug.x) + Math.abs(other.y - bug.y) <= (bug.species === 'ladybug' ? 3 : TERRARIUM_PREDATOR_SENSE_RADIUS) &&
    !(bug.species === 'spider' && other.species === 'aphid' && state.plants.some(p => p.species === 'mint' && p.stage >= 2 && p.health >= 60 && !p.dry && p.x === other.x && p.y === other.y) && (bug.x !== other.x || bug.y !== other.y)) &&
    (bug.species === 'spider' ? other.species !== 'spider' : other.species === 'aphid'));
  const target = [...targets].sort((a, b) =>
    Math.abs(a.x - bug.x) + Math.abs(a.y - bug.y) - Math.abs(b.x - bug.x) - Math.abs(b.y - bug.y) || a.id.localeCompare(b.id))[0];
  if (target) {
    const before = bug.species !== 'aphid' && Math.abs(bug.x - target.x) + Math.abs(bug.y - target.y) <= 1 ? captureFrame(state, currentPhase) : undefined;
    if (!step(Math.sign(target.x - bug.x), 0)) step(0, Math.sign(target.y - bug.y));
    if (bug.x === target.x && bug.y === target.y) {
      if (bug.species === 'aphid') { damage(target as PlantState, 3, '蚜虫取食'); bug.energy = Math.min(100, bug.energy + 15); bug.hunger = Math.max(0, (bug.hunger ?? 0) - 25); }
      else {
        const contact = captureFrame(state, currentPhase);
        recordDeath(state, target, recordedAtMs, `被 ${bug.id}（${bug.species}）捕食`, { phase: currentPhase, moisture: state.soil[target.y * capacity(state).size + target.x].moisture });
        state.bugs.splice(state.bugs.indexOf(target as BugState), 1);
        if (state.webs) state.webs = state.webs.filter(web => state.bugs.some(bug => bug.id === web.spiderId));
        bug.energy = Math.min(100, bug.energy + 30);
        bug.hunger = Math.max(0, (bug.hunger ?? 0) - 60);
        const replay: HuntReplay = { id: `hunt-${recordedAtMs}-${bug.id}-${target.id}`, recordedAtMs, hunterId: bug.id, preyId: target.id, frames: [before!, contact, captureFrame(state, currentPhase)] };
        state.replays = [...(state.replays ?? []), replay].slice(-TERRARIUM_MAX_REPLAYS);
        changes.push({ kind: 'hunt', text: `${label(bug)} 捕食了 ${label(target)}。回放 ID：${replay.id}` });
      }
    }
  } else {
    const direction = (Math.floor(bug.ageHours) + Number(bug.id.split('-')[1])) % 4;
    for (let offset = 0; offset < (predator ? 4 : 1); offset++) {
      const next = (direction + offset) % 4;
      if (step([1, 0, -1, 0][next], [0, 1, 0, -1][next])) break;
    }
  }
}

function weave(state: TerrariumState, spider: BugState, atMs: number, changes: Change[]): void {
  const owned = (state.webs ?? []).filter(web => web.spiderId === spider.id);
  if (owned.some(web => atMs - web.createdAtMs < HOUR_MS) || (state.webs ?? []).some(web => web.x === spider.x && web.y === spider.y) || isNestEntrance(state, spider.x, spider.y)) return;
  const web: SpiderWeb = { id: `web-${state.nextId++}`, spiderId: spider.id, x: spider.x, y: spider.y, createdAtMs: atMs };
  if (owned.length >= TERRARIUM_WEBS_PER_SPIDER) state.webs = state.webs!.filter(value => value.id !== owned[0].id);
  (state.webs ??= []).push(web);
  changes.push({ kind: 'web', text: `${label(spider)} 在 (${web.x},${web.y}) 织出蛛网 ${web.id}，飞虫触网会被捕食。` });
}

function trapFly(state: TerrariumState, fly: BugState, atMs: number, currentPhase: 'day' | 'night', changes: Change[], before = captureFrame(state, currentPhase)): boolean {
  const web = state.webs?.find(web => web.x === fly.x && web.y === fly.y);
  const spider = web && state.bugs.find(bug => bug.id === web.spiderId && bug.species === 'spider');
  if (!spider) return false;
  spider.x = fly.x; spider.y = fly.y;
  const contact = captureFrame(state, currentPhase);
  recordDeath(state, fly, atMs, `触网后被 ${spider.id} 捕食`, { phase: currentPhase, moisture: state.soil[fly.y * capacity(state).size + fly.x].moisture });
  state.bugs.splice(state.bugs.indexOf(fly), 1);
  spider.lastMeal = { atMs, kind: 'foraging' };
  spider.energy = Math.min(100, spider.energy + 30);
  spider.hunger = Math.max(0, (spider.hunger ?? 0) - 60);
  const replay: HuntReplay = { id: `hunt-${atMs}-${spider.id}-${fly.id}`, recordedAtMs: atMs, hunterId: spider.id, preyId: fly.id, frames: [before, contact, captureFrame(state, currentPhase)] };
  state.replays = [...(state.replays ?? []), replay].slice(-TERRARIUM_MAX_REPLAYS);
  changes.push({ kind: 'hunt', text: `${label(fly)} 飞入 ${web!.id}，被 ${label(spider)} 捕食。回放 ID：${replay.id}` });
  return true;
}

function actFly(state: TerrariumState, fly: BugState, atMs: number, currentPhase: 'day' | 'night', changes: Change[]): void {
  if (state.webs?.some(web => web.x === fly.x && web.y === fly.y) && trapFly(state, fly, atMs, currentPhase, changes)) return;
  const draw = flightDraw(fly.id, atMs);
  if (draw < TERRARIUM_FLY_MOVE_PROBABILITY) {
    const options = [[1, 0], [0, 1], [-1, 0], [0, -1]].map(([dx, dy]) => ({ x: fly.x + dx, y: fly.y + dy })).filter(p => inHabitat(state, p.x, p.y));
    const target = options[Math.floor(draw / TERRARIUM_FLY_MOVE_PROBABILITY * options.length)];
    const before = state.webs?.some(web => web.x === target.x && web.y === target.y) ? captureFrame(state, currentPhase) : undefined;
    fly.x = target.x; fly.y = target.y;
    if (before && trapFly(state, fly, atMs, currentPhase, changes, before)) return;
  }
  if (fly.energy < 70 || (fly.hunger ?? 0) >= TERRARIUM_BUG_FORAGE_HUNGER) {
    const plant = state.plants.find(plant => plant.x === fly.x && plant.y === fly.y && flowerStage(plant) === 'bloom');
    if (plant) { fly.energy = Math.min(100, fly.energy + 5); fly.hunger = Math.max(0, (fly.hunger ?? 0) - 15); }
  }
}

const phaseFormatters = new Map<string, Intl.DateTimeFormat>();

export function phase(atMs: number, timezone: string): 'day' | 'night' {
  let formatter = phaseFormatters.get(timezone);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat('en-GB', { timeZone: timezone, hour: '2-digit', hourCycle: 'h23' });
    phaseFormatters.set(timezone, formatter);
  }
  const parts = formatter.formatToParts(atMs);
  const hour = Number(parts.find(part => part.type === 'hour')?.value);
  return hour >= 6 && hour < 18 ? 'day' : 'night';
}

function recordHistory(state: TerrariumState, recordedAtMs: number): void {
  if (state.progression) observeProgress(state);
  state.history.push({ recordedAtMs, gridSize: capacity(state).size, score: score(state), scoreVersion: TERRARIUM_SCORE_VERSION, soilMoisture: state.soil.map(cell => cell.moisture), plants: state.plants.map(({ id, species, moisture, health, stage, growthHours }) => ({ id, species, moisture, health, stage, growthHours })) });
  state.history = state.history.filter(sample => sample.recordedAtMs >= recordedAtMs - TERRARIUM_MAX_HISTORY * HOUR_MS).slice(-TERRARIUM_MAX_HISTORY);
}

function advanceSoil(state: TerrariumState, hours: number, isDay: boolean): void {
  const moisture = state.soil.map(cell => cell.moisture);
  for (const cell of state.soil) {
    const index = cell.y * capacity(state).size + cell.x;
    let diffusion = 0;
    for (const [dx, dy] of [[-1, 0], [1, 0], [0, -1], [0, 1]]) {
      if (inHabitat(state, cell.x + dx, cell.y + dy)) diffusion += (moisture[(cell.y + dy) * capacity(state).size + cell.x + dx] - moisture[index]) * .08;
    }
    const light = daylight(state, cell.x, cell.y);
    const evaporation = (isDay ? .8 + light * .4 : .6) * (state.plants.some(p => p.species === 'moss' && p.health >= 60 && Math.abs(p.x - cell.x) + Math.abs(p.y - cell.y) <= 1) ? .85 : 1) * (hasFacility(state, 'mulch') ? .75 : 1);
    cell.moisture = Math.max(0, Math.min(100, moisture[index] + (diffusion - evaporation) * hours));
  }
}

export function advance(state: TerrariumState, nowMs: number, timezone: string): Change[] {
  spreadAnts(state);
  for (const egg of state.eggs ?? []) egg.hatchesAtMs = Math.min(egg.hatchesAtMs, egg.laidAtMs + TERRARIUM_BREEDING.incubationHours * HOUR_MS);
  ensureQueen(state);
  for (const egg of state.eggs ?? []) if (egg.species === 'ant' && !egg.inside) {
    const nest = state.nests?.find(nest => nest.x === egg.x && nest.y === egg.y);
    if (nest) egg.inside = { nestId: nest.id, ...TERRARIUM_NEST_NURSERY };
  }
  if (nowMs <= state.updatedAtMs) return [];
  const changes: Change[] = [];
  // Old saves begin hunger at the end of their first catch-up, without retroactive starvation.
  const legacyBugs = new Set(state.bugs.filter(bug => bug.hunger === undefined));
  const initialHealth = new Map(state.plants.map(plant => [plant.id, plant.health]));
  const losses = new Map<string, Map<string, number>>();
  const damage: PlantDamage = (plant, amount, cause) => {
    const loss = Math.min(plant.health, amount);
    plant.health -= loss;
    if (cause === '蚂蚁采集' && loss > 0) plant.leafDamage = Math.min(100, (plant.leafDamage ?? 0) + loss * TERRARIUM_LEAF_DAMAGE.perHarvestHealth);
    if (loss <= 0) return;
    const causes = losses.get(plant.id) ?? new Map<string, number>();
    causes.set(cause, (causes.get(cause) ?? 0) + loss);
    losses.set(plant.id, causes);
  };
  const damageText = (plant: PlantState): string => {
    const causes = losses.get(plant.id);
    return causes ? ` 本次结算损伤：${[...causes].map(([cause, loss]) => `${cause} -${loss.toFixed(2)} 健康`).join('；')}。` : '';
  };
  // The simulation catches up after downtime while keeping a finite work bound.
  const stepMs = state.bugs.some(bug => bug.species === 'ant' || isFlying(bug.species)) || state.eggs?.length ? TERRARIUM_ANT_ACTION_INTERVAL_MS : TERRARIUM_BUG_ACTION_INTERVAL_MS;
  const from = Math.max(state.updatedAtMs, nowMs - TERRARIUM_MAX_CATCHUP_MS);
  const irrigated = new Map<string, { count: number; added: number }>();
  for (let start = from; start < nowMs;) {
    ensureQueen(state);
    const due = (state.irrigation ?? []).map(s => Math.max(s.lastWateredAtMs, nowMs - TERRARIUM_MAX_CATCHUP_MS) + s.intervalMinutes * 60_000).filter(at => at > start);
    const end = Math.min(nowMs, (Math.floor(start / stepMs) + 1) * stepMs, ...due);
    if (state.webs) state.webs = state.webs.filter(web => end - web.createdAtMs < TERRARIUM_WEB_LIFETIME_MS && state.bugs.some(bug => bug.id === web.spiderId && bug.species === 'spider'));
    const hours = (end - start) / HOUR_MS;
    const before = phase(start, timezone);
    const after = phase(end, timezone);
    if (before !== after) changes.push({ kind: 'phase', text: after === 'day' ? '生态箱天亮了。' : '生态箱天黑了。' });
    advanceSoil(state, hours, before === 'day');
    for (const schedule of state.irrigation ?? []) {
      const intervalMs = schedule.intervalMinutes * 60_000;
      // Intervals older than the simulation bound are not replayed.
      const base = Math.max(schedule.lastWateredAtMs, nowMs - TERRARIUM_MAX_CATCHUP_MS);
      const count = Math.floor((end - base) / intervalMs);
      if (count < 1) continue;
      const cell = state.soil[schedule.y * capacity(state).size + schedule.x], previous = cell.moisture;
      waterSoil(state, schedule.x, schedule.y, { targetMoisture: schedule.targetMoisture });
      schedule.lastWateredAtMs = base + count * intervalMs;
      const key = `${schedule.x},${schedule.y}`, total = irrigated.get(key) ?? { count: 0, added: 0 };
      total.count += count; total.added += cell.moisture - previous; irrigated.set(key, total);
    }
    for (const seed of [...state.seeds]) {
      const moisture = state.soil[seed.y * capacity(state).size + seed.x].moisture;
      if (before === 'day' && moisture >= TERRARIUM_PLANT_REQUIREMENTS[seed.species].minMoisture) seed.ageHours += hours;
      if (seed.ageHours >= 2) {
        state.seeds.splice(state.seeds.indexOf(seed), 1);
        state.plants.push({ id: seed.id, species: seed.species, x: seed.x, y: seed.y, moisture, growthHours: 0, stage: 0, dry: false, health: 100 });
        changes.push({ kind: 'sprout', text: `${label(seed)} 发芽了。` });
      }
    }
    if (state.food) state.food = state.food.filter(food => food.droppedAtMs + 48 * HOUR_MS > end);
    for (const plant of state.plants) {
      const previousFlower = flowerStage(plant);
      plant.moisture = state.soil[plant.y * capacity(state).size + plant.x].moisture;
      const needs = TERRARIUM_PLANT_REQUIREMENTS[plant.species];
      const light = daylight(state, plant.x, plant.y);
      const hydrated = plant.moisture >= needs.minMoisture && plant.moisture <= needs.maxMoisture;
      if (before === 'day' && hydrated && light >= needs.minLight && light <= needs.maxLight) {
        plant.growthHours += hours * (plant.species === 'basil' && state.plants.some(p => p.species === 'bean' && p.health >= 60 && Math.abs(p.x - plant.x) + Math.abs(p.y - plant.y) === 1) ? 1.1 : 1);
        if (plant.leafDamage !== undefined && plant.health >= TERRARIUM_PLANT_RECOVERED_HEALTH) plant.leafDamage = Math.max(0, plant.leafDamage - hours * TERRARIUM_LEAF_DAMAGE.recoveryPerGrowthHour);
      }
      if (!hydrated) damage(plant, hours * .5, plant.moisture < needs.minMoisture ? '水分过低' : '水分过高');
      const stage = STAGES.reduce((n, threshold, index) => plant.growthHours >= threshold ? index : n, 0);
      if (stage !== plant.stage) {
        plant.stage = stage;
        changes.push({ kind: 'stage', text: `${label(plant)} 长到了第 ${stage} 阶段。` });
      }
      if (!plant.dry && plant.moisture < needs.minMoisture) {
        plant.dry = true;
        changes.push({ kind: 'dry', text: `${label(plant)} 的土壤变干了（水分 ${plant.moisture.toFixed(1)}，生长下限 ${needs.minMoisture}）。${plantCareAdvice(plant)}` });
      }
      plant.dry = plant.moisture < needs.minMoisture;
      if (before === 'day' && hydrated && light >= needs.minLight && light <= needs.maxLight && plant.health >= TERRARIUM_PLANT_RECOVERED_HEALTH && plant.species === 'sunflower' && plant.growthHours >= TERRARIUM_SUNFLOWER.seedHours) {
        const rounds = sunflowerSeedRounds(plant.growthHours);
        if (rounds > (plant.seedRounds ?? 0)) {
          plant.seedRounds = rounds;
          if ((state.food?.length ?? 0) < 20) {
            const food: LandFood = { id: `food-${state.nextId++}`, x: plant.x, y: plant.y, portions: TERRARIUM_SUNFLOWER.seedPortions, droppedAtMs: end, kind: 'sunflower_seed' };
            (state.food ??= []).push(food);
            changes.push({ kind: 'stage', text: `${label(plant)} 结籽，落下 ${TERRARIUM_SUNFLOWER.seedPortions} 份葵花籽 ${food.id}，蚂蚁可以采集；48 小时后腐解。` });
          }
        }
      }
      if (flowerStage(plant) === 'bloom' && previousFlower !== 'bloom') changes.push({ kind: 'stage', text: `${label(plant)} 开花了。` });
    }
    const hourlyStep = Math.floor(start / HOUR_MS) !== Math.floor(end / HOUR_MS);
    const currentBugs = [...state.bugs];
    if (hourlyStep && state.bugs.length + (state.eggs?.length ?? 0) < capacity(state).organisms) {
      const paired = new Set<string>();
      for (const bug of currentBugs) {
        if (state.bugs.length + (state.eggs?.length ?? 0) >= capacity(state).organisms) break;
        if (!state.bugs.includes(bug)) continue;
        if (bug.species === 'ant') continue;
        if (paired.has(bug.id) || bug.ageHours + hours + 1e-9 < TERRARIUM_BREEDING.maturityHours || Math.floor((bug.ageHours + 1e-9) / TERRARIUM_BREEDING.pairIntervalHours) === Math.floor((bug.ageHours + hours + 1e-9) / TERRARIUM_BREEDING.pairIntervalHours) || bug.energy - hours * 2 < 70) continue;
        const mate = currentBugs.find(other => other.id !== bug.id && state.bugs.includes(other) && !paired.has(other.id) && other.species === bug.species && other.x === bug.x && other.y === bug.y && other.ageHours + hours + 1e-9 >= TERRARIUM_BREEDING.maturityHours && other.energy - hours * 2 >= 70);
        if (!mate) continue;
        bug.energy -= 25; mate.energy -= 25;
        paired.add(bug.id); paired.add(mate.id);
        if (bug.species === 'aphid') {
          const child = { id: `bug-${state.nextId++}`, species: bug.species, x: bug.x, y: bug.y, ageHours: 0, energy: 45, hunger: 0 };
          state.bugs.push(child);
          changes.push({ kind: 'birth', text: `${bug.id} 和 ${mate.id} 产下幼虫 ${child.id}（aphid），位置 (${child.x},${child.y})。` });
        } else {
          const eggs = state.eggs ??= [];
          const count = Math.min(TERRARIUM_BREEDING.eggsPerClutch, capacity(state).organisms - state.bugs.length - eggs.length);
          for (let i = 0; i < count; i++) eggs.push({ id: `egg-${state.nextId++}`, species: bug.species, x: bug.x, y: bug.y, laidAtMs: end, hatchesAtMs: end + TERRARIUM_BREEDING.incubationHours * HOUR_MS });
          changes.push({ kind: 'egg', text: `${bug.id} 和 ${mate.id} 产下 ${count} 枚 ${bug.species} 卵，位置 (${bug.x},${bug.y})，${TERRARIUM_BREEDING.incubationHours} 小时后孵化。` });
        }
        if (state.bugs.length + (state.eggs?.length ?? 0) >= capacity(state).organisms) break;
      }
    }
    for (const bug of currentBugs) {
      if (!state.bugs.includes(bug)) continue;
      const previousEnergy = bug.energy;
      const previousHunger = bug.hunger ?? 0;
      bug.hunger = legacyBugs.has(bug) ? 0 : Math.min(100, previousHunger + hours * 100 / TERRARIUM_BUG_STARVATION_HOURS);
      if (bug.hunger >= 100 - 1e-9) {
        recordDeath(state, bug, end, '饥饿值达到 100', { phase: after, moisture: state.soil[bug.y * capacity(state).size + bug.x].moisture });
        state.bugs.splice(state.bugs.indexOf(bug), 1);
        if (state.webs) state.webs = state.webs.filter(web => web.spiderId !== bug.id);
        changes.push({ kind: 'death', text: `${label(bug)} 因饥饿死亡：饥饿值达到 100。` });
        continue;
      }
      bug.ageHours += hours; bug.energy = Math.max(0, bug.energy - hours * 2);
      const interval = isFlying(bug.species) ? TERRARIUM_FLY_ACTION_INTERVAL_MS : bug.species === 'ant' ? TERRARIUM_ANT_ACTION_INTERVAL_MS : TERRARIUM_BUG_ACTION_INTERVAL_MS;
      if (Math.floor(start / interval) !== Math.floor(end / interval)) {
        const hungerBeforeEating = bug.hunger;
        if (bug.species === 'spider') weave(state, bug, end, changes);
        if (isFlying(bug.species)) actFly(state, bug, end, after, changes);
        else if (bug.species === 'ant') { actAnt(state, bug, changes, damage); spreadAnts(state); }
        else actBug(state, bug, changes, end, after, damage);
        if ((bug.hunger ?? 0) < hungerBeforeEating) bug.lastMeal = { atMs: end, kind: 'foraging' };
      }
      if (!state.bugs.includes(bug)) continue;
      if (bug.role === 'queen' && bug.inside && bug.ageHours >= TERRARIUM_BREEDING.queenMaturityHours - 1e-9 && bug.ageHours < 120 && bug.energy >= 70 && Math.floor((bug.ageHours - hours + 1e-9) / TERRARIUM_BREEDING.queenIntervalHours) !== Math.floor((bug.ageHours + 1e-9) / TERRARIUM_BREEDING.queenIntervalHours)) {
        const eggs = state.eggs ??= [];
        const count = Math.min(TERRARIUM_BREEDING.eggsPerClutch, capacity(state).organisms - state.bugs.length - eggs.length);
        if (count > 0) {
          for (let i = 0; i < count; i++) eggs.push({ id: `egg-${state.nextId++}`, species: 'ant', x: bug.x, y: bug.y, inside: { ...bug.inside }, laidAtMs: end, hatchesAtMs: end + TERRARIUM_BREEDING.incubationHours * HOUR_MS });
          bug.energy -= 25;
          changes.push({ kind: 'egg', text: `蚁后 ${bug.id} 在蚁巢 ${bug.inside.nestId} 产下 ${count} 枚卵，${TERRARIUM_BREEDING.incubationHours} 小时后孵化。` });
        }
      }
      if (bug.ageHours >= 120) { recordDeath(state, bug, end, '寿命达到 120 小时', { phase: after, moisture: state.soil[bug.y * capacity(state).size + bug.x].moisture }); state.bugs.splice(state.bugs.indexOf(bug), 1); if (state.webs) state.webs = state.webs.filter(web => web.spiderId !== bug.id); changes.push({ kind: 'death', text: `${label(bug)} 因寿命达到 120 小时死亡。` }); }
      else if (previousHunger < TERRARIUM_BUG_HUNGER_WARNING && bug.hunger >= TERRARIUM_BUG_HUNGER_WARNING) changes.push({ kind: 'hungry', text: `${label(bug)} 饥饿值 ${bug.hunger.toFixed(1)}/100，达到 100 会饿死。需要实际取食；terrarium_feed_bug 只补能量，不能降低饥饿值。` });
      else if (previousEnergy > 20 && bug.energy <= 20) changes.push({ kind: 'hungry', text: `${bug.id}（${bug.species}）体力低于 20，位置 (${bug.x},${bug.y})。` });
    }
    for (const egg of [...(state.eggs ?? [])]) if (egg.hatchesAtMs <= end && state.bugs.length < capacity(state).organisms) {
      state.eggs!.splice(state.eggs!.indexOf(egg), 1);
      const child: BugState = { id: `bug-${state.nextId++}`, species: egg.species, x: egg.x, y: egg.y, ageHours: 0, energy: 45, hunger: 0 };
      if (egg.species === 'ant') { child.role = 'worker'; if (egg.inside) child.inside = { ...egg.inside }; }
      state.bugs.push(child); spreadAnts(state);
      changes.push({ kind: 'birth', text: `${egg.id} 孵化为幼虫 ${child.id}（${child.species}），位置 (${child.x},${child.y})。` });
    }
    for (const plant of [...state.plants]) {
      if (plant.health <= 0) { recordDeath(state, plant, end, damageText(plant) || '健康已耗尽', { phase: after, moisture: plant.moisture, light: daylight(state, plant.x, plant.y) }); state.plants.splice(state.plants.indexOf(plant), 1); changes.push({ kind: 'death', text: `${label(plant)} 枯死了。水分 ${plant.moisture.toFixed(1)}。${damageText(plant)}` }); }
      else if (plant.health <= TERRARIUM_PLANT_LOW_HEALTH && !plant.healthWarning) {
        plant.healthWarning = true;
        const needs = TERRARIUM_PLANT_REQUIREMENTS[plant.species];
        changes.push({ kind: 'health', text: `${plant.id}（${plant.species}）濒危：健康 ${plant.health.toFixed(1)}/100，位置 (${plant.x},${plant.y})，土壤水分 ${plant.moisture.toFixed(1)}（适宜 ${needs.minMoisture}–${needs.maxMoisture}）。${damageText(plant)}${plantCareAdvice(plant)}` });
      } else if (plant.health >= TERRARIUM_PLANT_RECOVERED_HEALTH) delete plant.healthWarning;
    }
    if (state.webs) state.webs = state.webs.filter(web => end - web.createdAtMs < TERRARIUM_WEB_LIFETIME_MS && state.bugs.some(bug => bug.id === web.spiderId && bug.species === 'spider'));
    if (hourlyStep) recordHistory(state, end);
    start = end;
  }
  for (const plant of state.plants) {
    const previous = initialHealth.get(plant.id);
    if (previous !== undefined && plant.health > TERRARIUM_PLANT_LOW_HEALTH && [75, 50, 40].some(threshold => previous > threshold && plant.health <= threshold)) {
      changes.push({ kind: 'health', text: `${plant.id}（${plant.species}）健康下降：${previous.toFixed(1)} → ${plant.health.toFixed(1)}/100，位置 (${plant.x},${plant.y})。${damageText(plant)}${plantCareAdvice(plant)}` });
    }
  }
  if (state.replays) state.replays = state.replays.filter(replay => replay.recordedAtMs >= nowMs - TERRARIUM_REPLAY_RETENTION_MS);
  state.updatedAtMs = nowMs;
  for (const [cell, total] of irrigated) changes.push({ kind: 'irrigation', text: `定时补水 (${cell})：执行 ${total.count} 次，合计补充 ${total.added.toFixed(1)} 水分，按模拟时间结算。` });
  return changes;
}

export function water(state: TerrariumState, id: PlantId, options: WateringOptions = {}): PlantState {
  const plant = state.plants.find(item => item.id === id);
  if (!plant) throw new Error('Unknown plant');
  waterSoil(state, plant.x, plant.y, options);
  return plant;
}

export function waterSoil(state: TerrariumState, x: number, y: number, options: WateringOptions = {}): SoilCell {
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  const cell = state.soil[y * capacity(state).size + x];
  validateWatering(options);
  const occupant = state.plants.find(p => p.x === x && p.y === y) ?? state.seeds.find(p => p.x === x && p.y === y);
  const minimum = occupant ? TERRARIUM_PLANT_REQUIREMENTS[occupant.species].minMoisture : 30;
  if (options.onlyDry && cell.moisture >= minimum) return cell;
  const added = waterAdded(cell.moisture, options);
  cell.moisture += added;
  if (added > 0) recordAction(state, 'care');
  const plant = state.plants.find(item => item.x === x && item.y === y);
  if (plant) {
    plant.moisture = cell.moisture;
    plant.dry = plant.moisture < TERRARIUM_PLANT_REQUIREMENTS[plant.species].minMoisture;
    plant.health = Math.min(100, plant.health + (added > 0 ? 5 : 0));
    if (plant.health >= TERRARIUM_PLANT_RECOVERED_HEALTH) delete plant.healthWarning;
  }
  return cell;
}

export function waterArea(state: TerrariumState, x1: number, y1: number, x2: number, y2: number, options: WateringOptions = {}): SoilCell[] {
  if (!inHabitat(state, x1, y1) || !inHabitat(state, x2, y2) || x1 > x2 || y1 > y2) throw new Error('Area must be ordered corners within the current habitat grid');
  validateWatering(options);
  const cells: SoilCell[] = [];
  for (let y = y1; y <= y2; y++) for (let x = x1; x <= x2; x++) cells.push(waterSoil(state, x, y, options));
  return cells;
}

export function move(state: TerrariumState, id: ItemId, x: number, y: number): ItemState {
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  if (state.plants.some(p => p.x === x && p.y === y)) throw new Error('That position belongs to a plant');
  const item = state.items.find(value => value.id === id);
  if (!item) throw new Error('Unknown item');
  if (state.items.some(other => other.id !== id && other.x === x && other.y === y)) throw new Error('That position is occupied');
  item.x = x; item.y = y;
  return item;
}

export async function loadState(file: string, nowMs: number): Promise<TerrariumState> {
  let raw: string;
  try { raw = await readFile(file, 'utf8'); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return newState(nowMs); throw error; }
  const state = migrateState(JSON.parse(raw)); spreadAnts(state); return state;
}

/** Reads earlier saves and reports without advancing their simulation time. */
export function migrateState(value: unknown): TerrariumState {
  if ((value as { version?: number })?.version === 1) {
    const old = value as { updatedAtMs: number; plants: Array<{ id: string; moisture: number; growthHours: number; stage: number; dry: boolean }>; items: ItemState[] };
    if (!Number.isFinite(old.updatedAtMs) || !Array.isArray(old.plants) || old.plants.length !== 2 || !Array.isArray(old.items)) throw new Error('Invalid terrarium state file');
    const migrated = newState(old.updatedAtMs);
    migrated.plants = old.plants.map(p => ({ ...p, species: p.id as PlantSpecies, x: p.id === 'fern' ? 0 : 4, y: 0, health: 100 }));
    migrated.items = old.items;
    if (!validState(migrated)) throw new Error('Invalid terrarium state file');
    initializeHabitat(migrated);
    return migrated;
  }
  if ((value as { version?: number })?.version === 2) {
    const migrated = { ...(value as TerrariumState), version: 3, soil: newState(0).soil, history: [] } as TerrariumState;
    if (!validState(migrated)) throw new Error('Invalid terrarium state file');
    initializeHabitat(migrated);
    return migrated;
  }
  if ((value as { version?: number })?.version === 3 && Array.isArray((value as TerrariumState).history)) {
    value = { ...(value as TerrariumState), history: (value as TerrariumState).history.map(sample => {
      if (sample && typeof sample === 'object' && !Object.hasOwn(sample, 'recordedAtMs') && Object.hasOwn(sample, 'atMs')) {
        const { atMs, ...rest } = sample as unknown as Record<string, unknown>;
        return { ...rest, recordedAtMs: atMs };
      }
      return sample;
    }) };
  }
  if (!validState(value)) throw new Error('Invalid terrarium state file');
  return value;
}

function initializeHabitat(state: TerrariumState): void {
  for (const plant of state.plants) {
    state.soil[plant.y * capacity(state).size + plant.x].moisture = plant.moisture;
    plant.dry = plant.moisture < TERRARIUM_PLANT_REQUIREMENTS[plant.species].minMoisture;
  }
  state.history = [];
  recordHistory(state, state.updatedAtMs);
}

function validHabitat(state: Partial<TerrariumState>): boolean {
  const percent = (value: number): boolean => Number.isFinite(value) && value >= 0 && value <= 100;
  return Array.isArray(state.soil) && state.soil.length === capacity(state).size ** 2 && state.soil.every((cell, i) => cell && cell.x === i % capacity(state).size && cell.y === Math.floor(i / capacity(state).size) && percent(cell.moisture) && (cell.allowAntForaging === undefined || typeof cell.allowAntForaging === 'boolean') && (cell.antBackfilled === undefined || typeof cell.antBackfilled === 'boolean')) &&
    Array.isArray(state.history) && state.history.length <= TERRARIUM_MAX_HISTORY && state.history.every((sample, i) => sample && Number.isFinite(sample.recordedAtMs) && (sample.scoreVersion === undefined || sample.scoreVersion === 1 || sample.scoreVersion === TERRARIUM_SCORE_VERSION) && (sample.score === undefined || Number.isSafeInteger(sample.score) && sample.score >= 0) && sample.recordedAtMs <= state.updatedAtMs! && (i === 0 || sample.recordedAtMs > state.history![i - 1].recordedAtMs) &&
      Array.isArray(sample.soilMoisture) && [5, 6, 7, 8].includes(sample.gridSize ?? 5) && (sample.gridSize ?? 5) <= capacity(state).size && sample.soilMoisture.length === (sample.gridSize ?? 5) ** 2 && sample.soilMoisture.every(percent) && Array.isArray(sample.plants) && sample.plants.length <= (TERRARIUM_LEVELS.find(l => l.size === (sample.gridSize ?? 5))?.organisms ?? 12) &&
      sample.plants.every(p => p && typeof p.id === 'string' && p.id.length <= 32 && Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS, p.species) && percent(p.moisture) && percent(p.health) && Number.isInteger(p.stage) && p.stage >= 0 && p.stage <= 3 && Number.isFinite(p.growthHours) && p.growthHours >= 0));
}

function validReplays(state: Partial<TerrariumState>): boolean {
  return state.replays === undefined || (Array.isArray(state.replays) && state.replays.length <= TERRARIUM_MAX_REPLAYS && state.replays.every((replay, i) => {
    if (!replay || typeof replay.id !== 'string' || replay.id.length > 160 || typeof replay.hunterId !== 'string' || typeof replay.preyId !== 'string' || !Number.isFinite(replay.recordedAtMs) || replay.recordedAtMs > state.updatedAtMs! || (i > 0 && replay.recordedAtMs < state.replays![i - 1].recordedAtMs) || !Array.isArray(replay.frames) || replay.frames.length !== 3) return false;
    if (!replay.frames.every(frame => frame && (frame.phase === 'day' || frame.phase === 'night') && validState({ ...frame, version: 3, nextId: state.nextId, updatedAtMs: replay.recordedAtMs, history: [], replays: undefined }))) return false;
    const [before, contact, after] = replay.frames;
    const hunter = contact.bugs.find(bug => bug.id === replay.hunterId);
    const prey = contact.bugs.find(bug => bug.id === replay.preyId);
    return !!hunter && !!prey && hunter.x === prey.x && hunter.y === prey.y && (hunter.species === 'spider' ? prey.species !== 'spider' : ['beetle', 'ladybug'].includes(hunter.species) && prey.species === 'aphid') && before.bugs.some(bug => bug.id === hunter.id) && before.bugs.some(bug => bug.id === prey.id) && after.bugs.some(bug => bug.id === hunter.id) && !after.bugs.some(bug => bug.id === prey.id);
  }) && new Set(state.replays.map(replay => replay.id)).size === state.replays.length);
}

export function validState(value: unknown): value is TerrariumState {
  if (!value || typeof value !== 'object') return false;
  const state = value as Partial<TerrariumState>;
  if (!validDevelopment(state, 'land') || !validCareAlerts(state.careAlerts) || !validDeaths(state.deaths, state.updatedAtMs!, capacity(state).size) || !validCollection(state.collection, 'land', capacity(state).collection) || (state.irrigation !== undefined && (!Array.isArray(state.irrigation) || state.irrigation.length > capacity(state).size ** 2 || new Set(state.irrigation.map(c => `${c?.x},${c?.y}`)).size !== state.irrigation.length || !state.irrigation.every(c => c && inHabitat(state, c.x, c.y) && Number.isFinite(c.targetMoisture) && c.targetMoisture >= 0 && c.targetMoisture <= 100 && Number.isFinite(c.intervalMinutes) && c.intervalMinutes >= 1 && c.intervalMinutes <= 1440 && Number.isFinite(c.lastWateredAtMs))))) return false;
  return state.version === 3 && (state.name === undefined || validName(state.name)) && validHabitat(state) && validReplays(state) && Number.isFinite(state.updatedAtMs) && Number.isSafeInteger(state.nextId) && state.nextId! >= 1 &&
    Array.isArray(state.plants) && state.plants.length <= capacity(state).organisms &&
    state.plants.every(p => p && typeof p.id === 'string' && p.id.length <= 32 && Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS, p.species) && inHabitat(state, p.x, p.y) && Number.isFinite(p.moisture) && p.moisture >= 0 && p.moisture <= 100 && Number.isFinite(p.growthHours) && p.growthHours >= 0 && Number.isInteger(p.stage) && p.stage >= 0 && p.stage <= 3 && typeof p.dry === 'boolean' && Number.isFinite(p.health) && p.health >= 0 && p.health <= 100 && (p.healthWarning === undefined || typeof p.healthWarning === 'boolean') && (p.leafDamage === undefined || (Number.isFinite(p.leafDamage) && p.leafDamage >= 0 && p.leafDamage <= 100)) && (p.seedRounds === undefined || (p.species === 'sunflower' && Number.isSafeInteger(p.seedRounds) && p.seedRounds >= 0 && p.seedRounds <= sunflowerSeedRounds(p.growthHours)))) &&
    Array.isArray(state.seeds) && state.seeds.length <= capacity(state).organisms && state.plants.length + state.seeds.length <= capacity(state).organisms && state.seeds.every(s => s && typeof s.id === 'string' && Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS, s.species) && inHabitat(state, s.x, s.y) && Number.isFinite(s.ageHours) && s.ageHours >= 0) &&
    Array.isArray(state.bugs) && state.bugs.length <= capacity(state).organisms && state.bugs.every(b => b && typeof b.id === 'string' && (['aphid', 'beetle', 'ladybug', 'spider', 'ant'].includes(b.species) || isFlying(b.species)) && inHabitat(state, b.x, b.y) && Number.isFinite(b.energy) && b.energy >= 0 && b.energy <= 100 && validMeal(b.lastMeal, state.updatedAtMs!) && (b.hunger === undefined || (Number.isFinite(b.hunger) && b.hunger >= 0 && b.hunger <= 100)) && (b.carrying === undefined || (b.species === 'ant' && Number.isInteger(b.carrying) && b.carrying >= 0 && b.carrying <= 5)) && Number.isFinite(b.ageHours) && b.ageHours >= 0) &&
    (state.eggs === undefined || (Array.isArray(state.eggs) && state.eggs.length <= capacity(state).organisms && state.eggs.every(egg => egg && typeof egg.id === 'string' && egg.id.length <= 32 && (['beetle', 'ladybug', 'spider', 'ant'].includes(egg.species) || isFlying(egg.species)) && inHabitat(state, egg.x, egg.y) && Number.isFinite(egg.laidAtMs) && egg.laidAtMs <= state.updatedAtMs! && Number.isFinite(egg.hatchesAtMs) && egg.hatchesAtMs > egg.laidAtMs && egg.hatchesAtMs <= egg.laidAtMs + 6 * HOUR_MS))) &&
    state.bugs.length + (state.eggs?.length ?? 0) <= capacity(state).organisms &&
    (state.nests === undefined || (Array.isArray(state.nests) && state.nests.length <= 5 && state.nests.every(nest => nest && typeof nest.id === 'string' && nest.id.length <= 32 && inHabitat(state, nest.x, nest.y) && Number.isInteger(nest.depth) && nest.depth >= 1 && nest.depth <= 5 && Number.isInteger(nest.food) && nest.food >= 0 && nest.food <= nest.depth * 20) && new Set(state.nests.map(nest => `${nest.x},${nest.y}`)).size === state.nests.length)) &&
    (state.food === undefined || (Array.isArray(state.food) && state.food.length <= 20 && state.food.every(food => food && typeof food.id === 'string' && food.id.length <= 32 && inHabitat(state, food.x, food.y) && Number.isInteger(food.portions) && food.portions >= 1 && food.portions <= 10 && Number.isFinite(food.droppedAtMs) && food.droppedAtMs <= state.updatedAtMs! && (food.kind === undefined || food.kind === 'sunflower_seed')))) &&
    (state.webs === undefined || (Array.isArray(state.webs) && state.webs.length <= capacity(state).organisms * TERRARIUM_WEBS_PER_SPIDER && state.webs.every(web => web && typeof web.id === 'string' && web.id.length <= 32 && typeof web.spiderId === 'string' && inHabitat(state, web.x, web.y) && Number.isFinite(web.createdAtMs) && web.createdAtMs <= state.updatedAtMs! && state.bugs!.some(bug => bug.id === web.spiderId && bug.species === 'spider')) && new Set(state.webs.map(web => `${web.x},${web.y}`)).size === state.webs.length && state.bugs.every(bug => state.webs!.filter(web => web.spiderId === bug.id).length <= TERRARIUM_WEBS_PER_SPIDER))) &&
    Array.isArray(state.items) && state.items.length === 2 &&
    ['coin', 'pebble'].every(id => state.items!.filter(item => item.id === id && inHabitat(state, item.x, item.y)).length === 1) &&
    !(state.items[0].x === state.items[1].x && state.items[0].y === state.items[1].y) &&
    new Set([...state.plants, ...state.seeds, ...state.bugs, ...(state.eggs ?? []), ...(state.nests ?? []), ...(state.food ?? []), ...(state.webs ?? []), ...(state.collection ?? []).map(e => e.specimen)].map(v => v.id)).size === (state.collection?.length ?? 0) + state.plants.length + state.seeds.length + state.bugs.length + (state.eggs?.length ?? 0) + (state.nests?.length ?? 0) + (state.food?.length ?? 0) + (state.webs?.length ?? 0) &&
    new Set([...state.plants, ...state.seeds].map(v => `${v.x},${v.y}`)).size === state.plants.length + state.seeds.length && validColony(state as TerrariumState);
}

export async function saveState(file: string, state: TerrariumState | import('./aquarium.ts').AquariumState): Promise<void> {
  await mkdir(dirname(file), { recursive: true });
  const temporary = `${file}.tmp`;
  await writeFile(temporary, JSON.stringify(state, null, 2), 'utf8');
  await rename(temporary, file);
}
