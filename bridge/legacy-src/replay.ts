import type { DevelopmentState } from './progression.ts';
import type { TerrariumState } from './state.ts';

export const TERRARIUM_MAX_REPLAYS = 16;
export const TERRARIUM_REPLAY_RETENTION_MS = 7 * 24 * 60 * 60_000;
export interface ReplayFrame extends DevelopmentState {
  phase: 'day' | 'night';
  soil: TerrariumState['soil'];
  plants: TerrariumState['plants'];
  seeds: TerrariumState['seeds'];
  bugs: TerrariumState['bugs'];
  items: TerrariumState['items'];
  eggs?: TerrariumState['eggs'];
  nests?: TerrariumState['nests'];
  food?: TerrariumState['food'];
  webs?: TerrariumState['webs'];
}
/** A hunt's movement, contact and result, copied from the simulation at the recorded time. */
export interface HuntReplay {
  id: string;
  recordedAtMs: number;
  hunterId: string;
  preyId: string;
  frames: [ReplayFrame, ReplayFrame, ReplayFrame];
}

export function captureFrame(state: TerrariumState, phase: ReplayFrame['phase']): ReplayFrame {
  return structuredClone({ progression: state.progression, facilities: state.facilities, phase, soil: state.soil, plants: state.plants, seeds: state.seeds, bugs: state.bugs, items: state.items, eggs: state.eggs, nests: state.nests, food: state.food, webs: state.webs });
}
