import type { AquariumState } from './aquarium.ts';
import type { TerrariumState } from './state.ts';
import { TERRARIUM_PLANT_REQUIREMENTS, TERRARIUM_PLANT_LOW_HEALTH, TERRARIUM_PLANT_RECOVERED_HEALTH } from './habitat.ts';

export interface CareAlertState { careAlerts?: Record<string, { severity: number; label: string }> }

export function validCareAlerts(value: unknown): boolean {
  return value === undefined || !!value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length <= 100 && Object.entries(value).every(([key, v]) => key.length <= 100 && v && [1, 2].includes(v.severity) && typeof v.label === 'string' && v.label.length <= 200);
}

/** Persisted levels suppress unchanged warnings across polling and restarts. */
export function careAlerts(state: TerrariumState | AquariumState): string[] {
  const previous = state.careAlerts ?? {}, current: NonNullable<CareAlertState['careAlerts']> = {};
  const living = new Set(['box', ...state.plants.map(p => p.id), ...('fish' in state ? state.fish : state.bugs).map(b => b.id)]);
  const add = (id: string, condition: string, severity: number, label: string): void => {
    if (severity) current[`${id}:${condition}`] = { severity, label };
  };
  const healthLevel = (id: string, health: number): number => health <= TERRARIUM_PLANT_LOW_HEALTH / 2 ? 2 : health <= TERRARIUM_PLANT_LOW_HEALTH || previous[`${id}:health`] && health < TERRARIUM_PLANT_RECOVERED_HEALTH ? 1 : 0;
  for (const p of state.plants) {
    const label = `${p.id}（${p.species}，${p.x},${p.y}）`;
    add(p.id, 'health', healthLevel(p.id, p.health), `${label} 健康 ${p.health.toFixed(1)}`);
    if ('moisture' in p) {
      const needs = TERRARIUM_PLANT_REQUIREMENTS[p.species];
      add(p.id, 'dry', p.moisture < needs.minMoisture ? p.moisture < needs.minMoisture / 2 ? 2 : 1 : 0, `${label} 缺水 ${p.moisture.toFixed(1)}，下限 ${needs.minMoisture}`);
      add(p.id, 'wet', p.moisture > needs.maxMoisture ? p.moisture > needs.maxMoisture + 10 ? 2 : 1 : 0, `${label} 过湿 ${p.moisture.toFixed(1)}，上限 ${needs.maxMoisture}`);
    }
  }
  for (const b of 'fish' in state ? state.fish : state.bugs) {
    const label = `${b.id}（${b.species}，${b.x},${b.y}）`;
    add(b.id, 'energy', b.energy <= 10 ? 2 : b.energy <= 20 ? 1 : 0, `${label} 能量 ${b.energy.toFixed(1)}`);
    if ('hunger' in b && typeof b.hunger === 'number') add(b.id, 'hunger', b.hunger >= 90 ? 2 : b.hunger >= 80 ? 1 : 0, `${label} 饥饿 ${b.hunger.toFixed(1)}；实际取食才能降低`);
    if ('health' in b) add(b.id, 'health', healthLevel(b.id, b.health), `${label} 健康 ${b.health.toFixed(1)}`);
  }
  if ('fish' in state) {
    add('box', 'oxygen', state.oxygen < 25 ? 2 : state.oxygen <= 40 || previous['box:oxygen'] && state.oxygen < 50 ? 1 : 0, `含氧 ${state.oxygen.toFixed(1)}，低于 25 损害鱼类健康`);
    add('box', 'cleanliness', state.cleanliness < 15 ? 2 : state.cleanliness < 30 ? 1 : 0, `水质 ${state.cleanliness.toFixed(1)}，低于 30 损害健康`);
  }
  const result: string[] = [];
  for (const [key, value] of Object.entries(current)) {
    const prior = previous[key];
    if (!prior || value.severity > prior.severity) result.push(`${prior ? '恶化' : '新增'}：${value.label}`);
  }
  const conditionNames: Record<string, string> = { dry: '缺水', wet: '过湿', health: '低健康', energy: '低能量', hunger: '饥饿', oxygen: '低含氧', cleanliness: '低水质' };
  for (const key of Object.keys(previous)) {
    const id = key.slice(0, key.lastIndexOf(':')), condition = key.slice(key.lastIndexOf(':') + 1);
    if (!current[key] && living.has(id)) result.push(`恢复：${id === 'box' ? '整箱' : id} ${conditionNames[condition]}已解除`);
  }
  state.careAlerts = current;
  return result;
}
