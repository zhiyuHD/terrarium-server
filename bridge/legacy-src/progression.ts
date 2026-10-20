import type { TerrariumState } from './state.ts';
import type { AquariumState } from './aquarium.ts';
import { score, scoreParts, TERRARIUM_SCORE_VERSION } from './scoring.ts';

export const TERRARIUM_LEVELS = [
  { level: 1, requiredScore: 0, size: 5, organisms: 12, collection: 48 },
  { level: 2, requiredScore: 120, size: 6, organisms: 18, collection: 64 },
  { level: 3, requiredScore: 260, size: 7, organisms: 24, collection: 96 },
  { level: 4, requiredScore: 420, size: 8, organisms: 32, collection: 128 },
] as const;
export const TERRARIUM_WATER_LEVELS = TERRARIUM_LEVELS.map((level, index) => ({ ...level, requiredScore: [0, 120, 240, 360][index] }));
export function expansionLevels(state: DevelopmentState) { return 'kind' in state && state.kind === 'water' ? TERRARIUM_WATER_LEVELS : TERRARIUM_LEVELS; }
export const TERRARIUM_FACILITIES = {
  mulch: { habitat: 'land', level: 2, name: '保湿覆盖', effect: '全箱土壤蒸发减少 25%；不改变渗透。' },
  shade: { habitat: 'land', level: 3, name: '遮阴棚', effect: '所在格与上下左右的日光乘以 0.6，可移动或关闭。' },
  grow_light: { habitat: 'land', level: 4, name: '育苗灯', effect: '白天将所在格与上下左右的光照提高到至少 0.75；夜晚仍不生长。' },
  air_stone: { habitat: 'water', level: 2, name: '气泡石', effect: '全箱每小时额外增加 2 氧气。' },
  filter: { habitat: 'water', level: 3, name: '过滤器', effect: '全箱每小时恢复 2 水质。' },
  water_light: { habitat: 'water', level: 4, name: '水草灯', effect: '白天水草生长速度提高 25%；夜晚仍不生长。' },
} as const;
export type FacilityKind = keyof typeof TERRARIUM_FACILITIES;
export interface Facility { kind: FacilityKind; x: number; y: number; enabled: boolean }
export interface Progression { scoreVersion?: number; level: number; peakScore: number; guideStep: number; guideSkipped: boolean; actions: string[] }
export interface DevelopmentState { progression?: Progression; facilities?: Facility[] }
type State = TerrariumState | AquariumState;
export function capacity(state: DevelopmentState) { return expansionLevels(state).find(l => l.level === state.progression?.level) ?? expansionLevels(state)[0]; }
export function inHabitat(state: DevelopmentState, x: number, y: number): boolean { const n = capacity(state).size; return Number.isInteger(x) && Number.isInteger(y) && x >= 0 && y >= 0 && x < n && y < n; }
export function hasFacility(state: DevelopmentState, kind: FacilityKind): boolean { return state.facilities?.some(f => f.kind === kind && f.enabled) ?? false; }
export function nearFacility(state: DevelopmentState, kind: FacilityKind, x: number, y: number): boolean { return state.facilities?.some(f => f.kind === kind && f.enabled && Math.abs(f.x - x) + Math.abs(f.y - y) <= 1) ?? false; }
export function observeProgress(state: State): Progression {
  const p = state.progression ??= { level: 1, peakScore: 0, guideStep: 0, guideSkipped: false, actions: [] };
  if (p.scoreVersion !== TERRARIUM_SCORE_VERSION) { p.peakScore = 0; p.scoreVersion = TERRARIUM_SCORE_VERSION; }
  p.peakScore = Math.max(p.peakScore, score(state), ...state.history.filter(h => h.scoreVersion === TERRARIUM_SCORE_VERSION).map(h => h.score ?? 0));
  return p;
}
const ACTIONS = ['observe', 'care', 'plant', 'inhabit', 'collect', 'upgrade', 'build'];
export function recordAction(state: State, action: string): void {
  const p = observeProgress(state);
  if (ACTIONS.includes(action) && !p.actions.includes(action)) p.actions.push(action);
}
export function upgradeHabitat(state: State): void {
  const p = observeProgress(state), next = expansionLevels(state)[p.level];
  if (!next) throw new Error('Habitat is already at the maximum level');
  if (p.peakScore < next.requiredScore) throw new Error(`Expansion requires a recorded peak score of ${next.requiredScore}; current peak ${p.peakScore}`);
  p.level++;
  if (!('fish' in state)) {
    const old = new Map(state.soil.map(c => [`${c.x},${c.y}`, c]));
    state.soil = Array.from({ length: next.size ** 2 }, (_, i) => { const x = i % next.size, y = Math.floor(i / next.size); return old.get(`${x},${y}`) ?? { x, y, moisture: 50 }; });
  }
  recordAction(state, 'upgrade');
}
export function installFacility(state: State, kind: unknown, x: unknown, y: unknown, enabled: unknown = true): void {
  if (typeof kind !== 'string' || !Object.hasOwn(TERRARIUM_FACILITIES, kind)) throw new Error('Unknown facility');
  const spec = TERRARIUM_FACILITIES[kind as FacilityKind];
  if (spec.habitat !== ('fish' in state ? 'water' : 'land')) throw new Error('Facility belongs to the other habitat');
  if (capacity(state).level < spec.level) throw new Error(`Facility unlocks at level ${spec.level}`);
  if (typeof x !== 'number' || typeof y !== 'number' || !inHabitat(state, x, y) || typeof enabled !== 'boolean') throw new Error('Supply valid x,y and boolean enabled');
  const entry = { kind: kind as FacilityKind, x, y, enabled };
  state.facilities = [...(state.facilities ?? []).filter(f => f.kind !== kind), entry];
  recordAction(state, 'build');
}
const LAND_LETTERS = [
  { title: '一封没有署名的来信', text: '欢迎接手这座小小的观察站。第一封信夹在箱盖里：“先看看这里的住客和环境，再决定怎样照顾它们。”主人可以在 Cortico 或 Coopanion 的生态箱面板查看画面。先用 inspect 的 soil、care 分区看水分、光照和适宜区间。宁可少浇，先确认需要再加水；水分高于物种上限或低于下限时，每个模拟小时损失 0.5 健康。植物与种子只在条件合适的白天累计生长，夜里等待天亮；石子会让所在格和上下左右的日光只剩 35%。温度目前没有单独模拟。查看后确认：soil 中每株所在格的水分和白天光照落在 requirements 对应区间；夜间 care.waitForDaylight 为 true 时，环境适宜就等待天亮。', task: '用 terrarium_inspect 查看地箱，soil 查环境与适宜范围，care 查生长条件。', action: 'observe' },
  { title: '水壶旁的便签', text: '旧水壶旁留着一行字：“合适比更多重要。”浇水默认是增加 35，amount 是加水量，targetMoisture 才是补到目标水分。fern 适宜水分 30–85、clover 20–80、moss 45–90、basil 30–75、bean 25–70；care 会给出每株适宜范围。先按这株植物的适宜区间选目标；过湿和缺水都会持续损失健康。onlyDry 会跳过已达到物种最低水分的格子，不是补满。浇水后看回执 watering.watered、植物 moistureStatus 与适宜范围；watered 为 0 表示没有加水，不必重复补满。', task: '按实际水分需要浇水一次；已经适宜就不必为了做任务反复浇。', action: 'care' },
  { title: '空白的植物标签', text: '抽屉里有几张空白标签。挑一格水分和光照合适的空地种下植物。种子需要累计 2 个适宜白天模拟小时才发芽，夜里不累计；同种只取贡献最高的前三株，按 100%、50%、25% 计分，种子本身不计分。多样化种植比挤满同一种更有用。播种后看回执 seed 或 seeds 的 ID 和坐标确认入土；之后用 care 看发芽剩余适宜白天模拟小时，夜间保持不变是正常的。', task: '在空格播种一次；可用 terrarium_sow_area 批量播种。', action: 'plant' },
  { title: '会动的邻居', text: '观察笔记还缺几页：谁吃什么，谁又需要庇护？先确认食物来源再放虫。蚜虫会啃植物，甲虫需要蚜虫，蜘蛛会捕食其他虫；蚂蚁缺粮也可能采集植物。能量与饥饿分开，feed_bug 只补能量，实际取食才能降低饥饿。放入蚂蚁并出现蚁巢后，用 inspect 的 colonies 找巢 ID，再用 terrarium_feed_nest 给巢添 5 份粮。放虫后用 inspect 的 residents 确认 ID，再用 care 的 feeding 看 lastMeal；饥饿下降才表示实际吃到了，单补能量不算。', task: '确认食物来源后放入一只虫子，观察其取食与饥饿。', action: 'inhabit' },
  { title: '旧仓库的钥匙', text: '柜底的钥匙终于找到了。收藏会暂停这位住客的模拟，也不再计分；其他生命仍需照顾。放回植物或种子需要一个没有植物和种子的格子。面板可先点选空格再点“放回选中格”，工具需要指定 x/y。收藏后看回执 collectionCount 和收藏列表确认标本；放回后查看回执坐标和 residents，确认它已回到箱内。', task: '收藏一位住客；放回时先选合法位置，位置不合适不会丢失收藏。', action: 'collect' },
  { title: '围栏外的新地方', text: '最后一封信写着：“当这里足够繁盛，围栏外还有地方。”扩建按新公式已记录的最高分判定，不花分，也不会因当前分下降缩小。地箱门槛为 120、260、420；旧公式高分不用于解锁。同种只计前三，动物养满 24 个模拟小时才有完整成熟度贡献，过饿会降分。扩建后用 terrarium_progress 确认 level、size 和 collection 容量已增加；最高分保留，不扣分。', task: '新公式最高分达到 120 后扩建到二级；剧情没有额外积分奖励。', action: 'upgrade' },
] as const;
const WATER_LETTERS = [
  { title: '一封没有署名的来信', text: '欢迎接手这座水下观察站。第一封信夹在箱盖里：“先看看这里的住客和环境，再决定怎样照顾它们。”主人可以在 Cortico 或 Coopanion 的生态箱·水面板查看。用 inspect 的 care 分区看含氧、水质和水草生长条件。水草只在条件适宜的白天生长，夜里等待天亮，同时继续消耗氧气；温度目前没有单独模拟。查看后确认：inspect 中氧气和水质的当前值，以及 care 中的生长条件和天亮含氧估计；夜间 waitForDaylight 为 true 时不用反复种植。', task: '用 terrarium_inspect 查看水箱和 care 照料建议。', action: 'observe' },
  { title: '水壶旁的便签', text: '便签上写着：“清水也需要留心照顾。”先检查氧气、水质与残饵；换水能改善环境，投饲料应按实际需要，吃不完的饲料会污染水质。鱼需要先游到食物处，已有残饵时等待也是正确操作，不必重复投放。氧气降至 40 会提醒，低于 25 损害鱼健康；水质至少 40 才能长草，低于 30 会损害鱼草健康。换水后看 inspect 的 oxygen、cleanliness；投饲料后看 food.portions 和鱼的 lastMeal，尚未游到时先等待，不要看到残饵就再加。', task: '按环境和饥饿需要换水或投放饲料一次。', action: 'care' },
  { title: '水下的植物标签', text: '挑一格空位，给水草、睡莲或海藻挂上标签。水草需要适宜水质和白天模拟时间；同种只计贡献最高的前三株，权重 100%、50%、25%。水环境分最高 40，按最低的氧气或水质比例，再乘以植物分/40、鱼分/40 与 1 中的最小值；只有水干净、没有鱼草，并不能拿满环境分。种植后用 inspect 的 residents 确认物种和坐标，再看 care 的 canGrowNow 与 suitableHoursRemaining；夜间等待天亮。', task: '种植一种水草、睡莲或海藻。', action: 'plant' },
  { title: '会游泳的邻居', text: '先看看食物，再决定请谁住进来。孔雀鱼与金鱼吃饲料，金鱼也会吃海藻；鲈鱼会捕食孔雀鱼。留心氧气、水质、健康与能量，鱼需要养满 24 个模拟小时才有完整成熟度贡献。放鱼后用 inspect 的 residents 确认鱼的 ID、健康与能量，再用 care 的 feeding.lastMeal 查看实际进食记录。', task: '确认食物和氧气条件后放入一条鱼。', action: 'inhabit' },
  { title: '旧仓库的钥匙', text: '收藏可以暂停一条鱼或一株水草的模拟，也暂停它的积分贡献。放回水草需要没有其他水草的格子；先在面板选位置再放回，或在工具中给出 x/y。水箱进度独立保存，内容围绕水下照料展开，可以单独跳过。收藏后查 collection 列表确认标本；放回后用 inspect 的 residents 确认鱼或水草已回到指定位置。', task: '收藏一位水下住客；按需选位置放回。', action: 'collect' },
  { title: '玻璃外的新地方', text: '观察站繁盛起来，水下也可以拥有更宽敞的空间。水箱扩建门槛为新公式最高分 120、240、360，按已记录最高分判定，不花分。旧公式高分不用于解锁，分数下降不会缩回已经扩建的空间。扩建后用 terrarium_progress 确认 level、size 和 collection 容量已增加；水箱进度独立于地箱。', task: '新公式最高分达到 120 后扩建到二级；剧情没有额外积分奖励。', action: 'upgrade' },
] as const;
const letters = (state: State) => 'fish' in state ? WATER_LETTERS : LAND_LETTERS;
export function guide(state: State) {
  const p = state.progression ?? { guideStep: 0, guideSkipped: false, actions: [] as string[] };
  const chapters = letters(state), chapter = chapters[p.guideStep];
  return { step: p.guideStep, total: chapters.length, skipped: p.guideSkipped, completed: p.guideStep >= chapters.length, ...(chapter && !p.guideSkipped ? { ...chapter, ready: p.actions.includes(chapter.action) } : {}) };
}
export function advanceGuide(state: State, expectedStep?: number): boolean {
  const p = observeProgress(state), chapter = letters(state)[p.guideStep];
  if (expectedStep !== undefined && (!Number.isInteger(expectedStep) || expectedStep < 0 || expectedStep > LAND_LETTERS.length)) throw new Error('Guide step must be a valid chapter index');
  if (p.guideSkipped || !chapter || (expectedStep !== undefined && expectedStep !== p.guideStep) || !p.actions.includes(chapter.action)) return false;
  p.guideStep++;
  return true;
}
export function development(state: State) {
  const c = capacity(state), peakScore = Math.max(state.progression?.scoreVersion === TERRARIUM_SCORE_VERSION ? state.progression.peakScore : 0, score(state), ...state.history.filter(h => h.scoreVersion === TERRARIUM_SCORE_VERSION).map(h => h.score ?? 0)), next = expansionLevels(state).find(l => l.level === c.level + 1);
  return { ...c, scoreParts: scoreParts(state), peakScore, next: next ? { ...next, remainingScore: Math.max(0, next.requiredScore - peakScore), available: peakScore >= next.requiredScore } : null, facilities: Object.entries(TERRARIUM_FACILITIES).filter(([, f]) => f.habitat === ('fish' in state ? 'water' : 'land')).map(([kind, f]) => ({ kind, ...f, unlocked: c.level >= f.level, installed: state.facilities?.find(e => e.kind === kind) ?? null })), guide: guide(state) };
}
export function validDevelopment(value: DevelopmentState, habitat: 'land' | 'water'): boolean {
  const p = value.progression;
  if (p !== undefined && (!p || (p.scoreVersion !== undefined && p.scoreVersion !== 1 && p.scoreVersion !== TERRARIUM_SCORE_VERSION) || !TERRARIUM_LEVELS.some(l => l.level === p.level && Number.isSafeInteger(p.peakScore) && p.peakScore >= 0) || !Number.isInteger(p.guideStep) || p.guideStep < 0 || p.guideStep > LAND_LETTERS.length || typeof p.guideSkipped !== 'boolean' || !Array.isArray(p.actions) || p.actions.length > ACTIONS.length || new Set(p.actions).size !== p.actions.length || !p.actions.every(a => ACTIONS.includes(a)))) return false;
  return value.facilities === undefined || Array.isArray(value.facilities) && value.facilities.length <= 3 && new Set(value.facilities.map(f => f?.kind)).size === value.facilities.length && value.facilities.every(f => f && Object.hasOwn(TERRARIUM_FACILITIES, f.kind) && TERRARIUM_FACILITIES[f.kind].habitat === habitat && capacity(value).level >= TERRARIUM_FACILITIES[f.kind].level && inHabitat(value, f.x, f.y) && typeof f.enabled === 'boolean');
}

export function guideHint(state: State) {
  const current = guide(state);
  if (current.completed || current.skipped || !('title' in current)) return undefined;
  return { step: current.step, title: current.title, task: current.task, ready: current.ready, next: { tool: 'terrarium_progress', habitat: 'fish' in state ? 'water' : 'land', action: current.ready ? 'guide' : 'status', step: current.step } };
}
