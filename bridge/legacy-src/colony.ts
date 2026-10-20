import { capacity, inHabitat } from './progression.ts';
import type { BugState, EggState, NestState, TerrariumState } from './state.ts';

export interface NestPosition { nestId: string; x: number; y: number }
export const TERRARIUM_NEST_SIZE = 4;
export const TERRARIUM_NEST_MAX_SIZE = 8;
export const TERRARIUM_NEST_DIG_ACTIONS = 12;
export const nestSize = (nest: NestState): number => nest.size ?? TERRARIUM_NEST_SIZE;
export const nestEntrances = (nest: NestState): Array<{ x: number; y: number }> => [{ x: nest.x, y: nest.y }, ...(nest.entrances ?? [])];
export const isNestEntrance = (state: Pick<TerrariumState, 'nests'>, x: number, y: number): boolean => (state.nests ?? []).some(nest => nestEntrances(nest).some(p => p.x === x && p.y === y));

export function emptyEntrancePlot(state: TerrariumState, x: number, y: number, builder?: BugState): boolean {
  return inHabitat(state, x, y) && !state.soil[y * capacity(state).size + x].antBackfilled && !isNestEntrance(state, x, y) &&
    ![...state.plants, ...state.seeds, ...state.items, ...(state.food ?? []), ...(state.webs ?? []), ...(state.eggs ?? []), ...(state.facilities ?? []), ...state.bugs.filter(b => !b.inside && b !== builder)].some(p => p.x === x && p.y === y);
}

const distance = (a: { x: number; y: number }, b: { x: number; y: number }) => Math.abs(a.x - b.x) + Math.abs(a.y - b.y);
function nearestFree(size: number, origin: { x: number; y: number }, occupied: Set<string>) {
  return Array.from({ length: size * size }, (_, i) => ({ x: i % size, y: Math.floor(i / size) }))
    .filter(p => !occupied.has(`${p.x},${p.y}`)).sort((a, b) => distance(a, origin) - distance(b, origin) || a.y - b.y || a.x - b.x)[0];
}

/** Preserve residents, reserving existing distinct cells before spreading overlaps. Overflow returns to the surface, whose cells outnumber active bug slots at every level. */
export function spreadAnts(state: TerrariumState): void {
  const ants = state.bugs.filter(b => b.species === 'ant');
  for (const nest of state.nests ?? []) {
    const residents = ants.filter(b => b.inside?.nestId === nest.id);
    const used = new Set<string>(), displaced: BugState[] = [];
    for (const ant of residents) {
      const p = ant.inside!, key = `${p.x},${p.y}`;
      if (used.has(key)) displaced.push(ant); else used.add(key);
    }
    for (const ant of displaced) {
      const free = nearestFree(nestSize(nest), ant.inside!, used);
      if (free) { Object.assign(ant.inside!, free); used.add(`${free.x},${free.y}`); }
      else { ant.x = nest.x; ant.y = nest.y; delete ant.inside; }
    }
  }
  const used = new Set<string>(), displaced: BugState[] = [];
  for (const ant of ants.filter(b => !b.inside)) {
    const key = `${ant.x},${ant.y}`;
    if (used.has(key)) displaced.push(ant); else used.add(key);
  }
  for (const ant of displaced) {
    const free = nearestFree(capacity(state).size, ant, used)!;
    Object.assign(ant, free); used.add(`${free.x},${free.y}`);
  }
}

export function surfaceAntOccupied(state: TerrariumState, x: number, y: number, except?: BugState): boolean {
  return state.bugs.some(b => b !== except && b.species === 'ant' && !b.inside && b.x === x && b.y === y);
}

export function fillAntEntrance(state: TerrariumState, x: number, y: number) {
  if (!inHabitat(state, x, y)) throw new Error('Position must be in the current habitat grid');
  const nest = state.nests?.find(n => nestEntrances(n).some(p => p.x === x && p.y === y));
  if (!nest) throw new Error('No ant entrance at this position');
  const remaining = nestEntrances(nest).filter(p => p.x !== x || p.y !== y);
  state.soil[y * capacity(state).size + x].antBackfilled = true;
  const residents = [...state.bugs, ...(state.eggs ?? [])].filter(b => b.inside?.nestId === nest.id);
  if (remaining.length) {
    Object.assign(nest, remaining[0]); nest.entrances = remaining.slice(1);
    for (const b of residents) { b.x = nest.x; b.y = nest.y; }
  } else {
    for (const b of residents) { b.x = x; b.y = y; delete b.inside; }
    state.nests!.splice(state.nests!.indexOf(nest), 1);
  }
  spreadAnts(state);
  return { x, y, nestId: nest.id, remainingEntrances: remaining.length, removedNest: !remaining.length, evacuatedAnts: remaining.length ? 0 : residents.filter(b => 'energy' in b).length, evacuatedEggs: remaining.length ? 0 : residents.filter(b => 'hatchesAtMs' in b).length };
}

/** A crowded colony adds one row and column after twelve worker digging actions, up to 8×8. */
export function digColony(state: TerrariumState, nest: NestState): { expanded: boolean; entrance?: { x: number; y: number } } {
  const size = nestSize(nest);
  const population = state.bugs.filter(b => b.species === 'ant').length;
  let expanded = false;
  if ((size >= TERRARIUM_NEST_MAX_SIZE || population < Math.ceil(size * size / 2)) && nest.digProgress !== undefined) nest.digProgress = 0;
  if (size < TERRARIUM_NEST_MAX_SIZE && population >= Math.ceil(size * size / 2)) {
    nest.digProgress = (nest.digProgress ?? 0) + 1;
    if (nest.digProgress >= TERRARIUM_NEST_DIG_ACTIONS) { nest.size = size + 1; nest.digProgress = 0; expanded = true; }
  }
  if (nestEntrances(nest).length >= nestSize(nest) - 3) return { expanded };
  for (const p of nestEntrances(nest)) for (const [dx, dy] of [[1, 0], [0, 1], [-1, 0], [0, -1]]) {
    const entrance = { x: p.x + dx, y: p.y + dy };
    if (emptyEntrancePlot(state, entrance.x, entrance.y)) { (nest.entrances ??= []).push(entrance); return { expanded, entrance }; }
  }
  return { expanded };
}

export const TERRARIUM_NEST_ENTRANCE = { x: 0, y: 0 };
export const TERRARIUM_NEST_STORAGE = { x: 3, y: 0 };
export const TERRARIUM_NEST_NURSERY = { x: 2, y: 2 };

/** One surviving ant becomes queen when the colony has no queen; the choice is saved with the ant. */
export function ensureQueen(state: TerrariumState, random: () => number = Math.random): void {
  const ants = state.bugs.filter(bug => bug.species === 'ant');
  if (!ants.length) return;
  const queen = ants.find(bug => bug.role === 'queen') ?? ants[Math.floor(random() * ants.length)];
  for (const ant of ants) ant.role = ant === queen ? 'queen' : 'worker';
}

export function enterNest(bug: BugState, nest: NestState): void {
  bug.x = nest.x; bug.y = nest.y;
  bug.inside = { nestId: nest.id, ...TERRARIUM_NEST_ENTRANCE };
}

export function walkInside(position: NestPosition, target: { x: number; y: number }): boolean {
  if (position.x !== target.x) position.x += Math.sign(target.x - position.x);
  else if (position.y !== target.y) position.y += Math.sign(target.y - position.y);
  return position.x === target.x && position.y === target.y;
}

export function nestContents<B extends { id: string; species: string; role?: 'queen' | 'worker'; inside?: NestPosition }, E extends { id: string; species: string; inside?: NestPosition }>(state: { bugs: B[]; eggs?: E[] }, nest: NestState) {
  const ants = state.bugs.filter(bug => bug.species === 'ant' && bug.inside?.nestId === nest.id);
  const eggs = (state.eggs ?? []).filter(egg => egg.species === 'ant' && egg.inside?.nestId === nest.id);
  return { ants, eggs, queens: ants.filter(ant => ant.role === 'queen').length, food: nest.food };
}

export function colonySnapshot(state: TerrariumState, nest: NestState) {
  const contents = nestContents(state, nest);
  return {
    id: nest.id, size: nestSize(nest), entrances: nestEntrances(nest), digProgress: nest.digProgress ?? 0, ...contents, antCount: contents.ants.length, eggCount: contents.eggs.length,
    cells: Array.from({ length: nestSize(nest) ** 2 }, (_, index) => {
      const x = index % nestSize(nest), y = Math.floor(index / nestSize(nest));
      return {
        x, y,
        ants: contents.ants.filter(ant => ant.inside!.x === x && ant.inside!.y === y).map(ant => ant.id),
        eggs: contents.eggs.filter(egg => egg.inside!.x === x && egg.inside!.y === y).map(egg => egg.id),
        food: x === TERRARIUM_NEST_STORAGE.x && y === TERRARIUM_NEST_STORAGE.y ? nest.food : 0,
      };
    }),
  };
}

/** Optional location fields preserve version-3 saves and reports from clients without colonies. */
export function validColony(state: TerrariumState): boolean {
  const nests = state.nests ?? [];
  const validResident = (resident: BugState | EggState): boolean => {
    if (resident.inside === undefined) return true;
    const p = resident.inside;
    const nest = nests.find(nest => nest.id === p?.nestId);
    return resident.species === 'ant' && !!nest && resident.x === nest.x && resident.y === nest.y &&
      Number.isInteger(p.x) && p.x >= 0 && p.x < nestSize(nest!) && Number.isInteger(p.y) && p.y >= 0 && p.y < nestSize(nest!);
  };
  if (!nests.every(n => (n.size === undefined || (Number.isInteger(n.size) && n.size >= TERRARIUM_NEST_SIZE && n.size <= TERRARIUM_NEST_MAX_SIZE)) &&
    (n.digProgress === undefined || (Number.isInteger(n.digProgress) && n.digProgress >= 0 && n.digProgress < TERRARIUM_NEST_DIG_ACTIONS)) &&
    (n.entrances === undefined || (Array.isArray(n.entrances) && n.entrances.length <= nestSize(n) - 4 && n.entrances.every(p => p && inHabitat(state, p.x, p.y)))))) return false;
  const entrances = nests.flatMap(nestEntrances);
  if (new Set(entrances.map(p => `${p.x},${p.y}`)).size !== entrances.length) return false;
  return state.bugs.every(bug => (bug.role === undefined || (bug.species === 'ant' && ['queen', 'worker'].includes(bug.role))) && validResident(bug)) &&
    state.bugs.filter(bug => bug.role === 'queen').length <= 1 && (state.eggs ?? []).every(validResident);
}
