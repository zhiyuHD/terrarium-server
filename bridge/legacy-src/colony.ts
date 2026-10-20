import type { BugState, EggState, NestState, TerrariumState } from './state.ts';

export interface NestPosition { nestId: string; x: number; y: number }
export const TERRARIUM_NEST_SIZE = 4;
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
    id: nest.id, size: TERRARIUM_NEST_SIZE, ...contents, antCount: contents.ants.length, eggCount: contents.eggs.length,
    cells: Array.from({ length: TERRARIUM_NEST_SIZE ** 2 }, (_, index) => {
      const x = index % TERRARIUM_NEST_SIZE, y = Math.floor(index / TERRARIUM_NEST_SIZE);
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
      Number.isInteger(p.x) && p.x >= 0 && p.x < TERRARIUM_NEST_SIZE && Number.isInteger(p.y) && p.y >= 0 && p.y < TERRARIUM_NEST_SIZE;
  };
  return state.bugs.every(bug => (bug.role === undefined || (bug.species === 'ant' && ['queen', 'worker'].includes(bug.role))) && validResident(bug)) &&
    state.bugs.filter(bug => bug.role === 'queen').length <= 1 && (state.eggs ?? []).every(validResident);
}
