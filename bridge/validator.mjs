// ../terrarium-server/bridge/worker.ts
import { createInterface } from "node:readline";

// ../terrarium-server/bridge/legacy-src/scoring.ts
var TERRARIUM_SCORE_VERSION = 2;
function speciesScore(organisms, value) {
  const groups = /* @__PURE__ */ new Map();
  for (const organism of organisms) {
    const values = groups.get(organism.species) ?? [];
    values.push(value(organism));
    groups.set(organism.species, values);
  }
  return Math.round([...groups.values()].reduce((sum, values) => sum + values.sort((a, b) => b - a).slice(0, 3).reduce((total, v, i) => total + v * [1, 0.5, 0.25][i], 0), 0));
}
var maturity = (ageHours) => 0.25 + 0.75 * Math.min(1, ageHours / 24);
function scoreParts(state) {
  const plants = speciesScore(state.plants, (plant) => (plant.stage + 1) * plant.health / 10);
  if ("fish" in state) {
    const fish = speciesScore(state.fish, (fish2) => 40 * fish2.health / 100 * fish2.energy / 100 * maturity(fish2.ageHours) * Math.min(1, state.oxygen / 50, state.cleanliness / 50));
    return { plants, fish, water: Math.round(40 * Math.min(state.oxygen, state.cleanliness) / 100 * Math.min(1, plants / 40, fish / 40)) };
  }
  return {
    plants,
    bugs: speciesScore(state.bugs, (bug) => 40 * bug.energy / 100 * (1 - (bug.hunger ?? 0) / 100) * maturity(bug.ageHours)),
    seeds: 0
  };
}
function score(state) {
  return Object.values(scoreParts(state)).reduce((sum, part) => sum + part, 0);
}

// ../terrarium-server/bridge/legacy-src/progression.ts
var TERRARIUM_LEVELS = [
  { level: 1, requiredScore: 0, size: 5, organisms: 12, collection: 48 },
  { level: 2, requiredScore: 120, size: 6, organisms: 18, collection: 64 },
  { level: 3, requiredScore: 260, size: 7, organisms: 24, collection: 96 },
  { level: 4, requiredScore: 420, size: 8, organisms: 32, collection: 128 }
];
var TERRARIUM_WATER_LEVELS = TERRARIUM_LEVELS.map((level, index) => ({ ...level, requiredScore: [0, 120, 240, 360][index] }));
function expansionLevels(state) {
  return "kind" in state && state.kind === "water" ? TERRARIUM_WATER_LEVELS : TERRARIUM_LEVELS;
}
var TERRARIUM_FACILITIES = {
  compost_bin: { habitat: "land", level: 2, name: "\u5806\u80A5\u6876", effect: "\u6BCF\u682A\u690D\u7269\u6362 1 \u4EFD\u80A5\u6599\uFF1B\u6BCF\u4EFD\u7ACB\u5373\u589E\u52A0\u6307\u5B9A\u683C\u4F5C\u7269 2 \u5C0F\u65F6\u751F\u957F\uFF0C\u53EF\u8FDE\u7EED\u65BD\u7528\uFF0C\u65E0\u51B7\u5374\u3002" },
  mulch: { habitat: "land", level: 2, name: "\u4FDD\u6E7F\u8986\u76D6", effect: "\u5168\u7BB1\u571F\u58E4\u84B8\u53D1\u51CF\u5C11 25%\uFF1B\u4E0D\u6539\u53D8\u6E17\u900F\u3002" },
  shade: { habitat: "land", level: 3, name: "\u906E\u9634\u68DA", effect: "\u6240\u5728\u683C\u4E0E\u4E0A\u4E0B\u5DE6\u53F3\u7684\u65E5\u5149\u4E58\u4EE5 0.6\uFF0C\u53EF\u79FB\u52A8\u6216\u5173\u95ED\u3002" },
  grow_light: { habitat: "land", level: 4, name: "\u80B2\u82D7\u706F", effect: "\u767D\u5929\u5C06\u6240\u5728\u683C\u4E0E\u4E0A\u4E0B\u5DE6\u53F3\u7684\u5149\u7167\u63D0\u9AD8\u5230\u81F3\u5C11 0.75\uFF1B\u591C\u665A\u4ECD\u4E0D\u751F\u957F\u3002" },
  air_stone: { habitat: "water", level: 2, name: "\u6C14\u6CE1\u77F3", effect: "\u5168\u7BB1\u6BCF\u5C0F\u65F6\u989D\u5916\u589E\u52A0 2 \u6C27\u6C14\u3002" },
  filter: { habitat: "water", level: 3, name: "\u8FC7\u6EE4\u5668", effect: "\u5168\u7BB1\u6BCF\u5C0F\u65F6\u6062\u590D 2 \u6C34\u8D28\u3002" },
  water_light: { habitat: "water", level: 4, name: "\u6C34\u8349\u706F", effect: "\u767D\u5929\u6C34\u8349\u751F\u957F\u901F\u5EA6\u63D0\u9AD8 25%\uFF1B\u591C\u665A\u4ECD\u4E0D\u751F\u957F\u3002" }
};
function capacity(state) {
  return expansionLevels(state).find((l) => l.level === state.progression?.level) ?? expansionLevels(state)[0];
}
function inHabitat(state, x, y) {
  const n = capacity(state).size;
  return Number.isInteger(x) && Number.isInteger(y) && x >= 0 && y >= 0 && x < n && y < n;
}
function observeProgress(state) {
  const p = state.progression ??= { level: 1, peakScore: 0, guideStep: 0, guideSkipped: false, actions: [] };
  if (p.scoreVersion !== TERRARIUM_SCORE_VERSION) {
    p.peakScore = 0;
    p.scoreVersion = TERRARIUM_SCORE_VERSION;
  }
  p.peakScore = Math.max(p.peakScore, score(state), ...state.history.filter((h) => h.scoreVersion === TERRARIUM_SCORE_VERSION).map((h) => h.score ?? 0));
  return p;
}
var ACTIONS = ["observe", "care", "plant", "inhabit", "collect", "upgrade", "build"];
function recordAction(state, action) {
  const p = observeProgress(state);
  if (ACTIONS.includes(action) && !p.actions.includes(action)) p.actions.push(action);
}
var LAND_LETTERS = [
  { title: "\u4E00\u5C01\u6CA1\u6709\u7F72\u540D\u7684\u6765\u4FE1", text: "\u6B22\u8FCE\u63A5\u624B\u8FD9\u5EA7\u5C0F\u5C0F\u7684\u89C2\u5BDF\u7AD9\u3002\u7B2C\u4E00\u5C01\u4FE1\u5939\u5728\u7BB1\u76D6\u91CC\uFF1A\u201C\u5148\u770B\u770B\u8FD9\u91CC\u7684\u4F4F\u5BA2\u548C\u73AF\u5883\uFF0C\u518D\u51B3\u5B9A\u600E\u6837\u7167\u987E\u5B83\u4EEC\u3002\u201D\u4E3B\u4EBA\u53EF\u4EE5\u5728 Cortico \u6216 Coopanion \u7684\u751F\u6001\u7BB1\u9762\u677F\u67E5\u770B\u753B\u9762\u3002\u5148\u7528 inspect \u7684 soil\u3001care \u5206\u533A\u770B\u6C34\u5206\u3001\u5149\u7167\u548C\u9002\u5B9C\u533A\u95F4\u3002\u5B81\u53EF\u5C11\u6D47\uFF0C\u5148\u786E\u8BA4\u9700\u8981\u518D\u52A0\u6C34\uFF1B\u6C34\u5206\u9AD8\u4E8E\u7269\u79CD\u4E0A\u9650\u6216\u4F4E\u4E8E\u4E0B\u9650\u65F6\uFF0C\u6BCF\u4E2A\u6A21\u62DF\u5C0F\u65F6\u635F\u5931 0.5 \u5065\u5EB7\u3002\u690D\u7269\u4E0E\u79CD\u5B50\u53EA\u5728\u6761\u4EF6\u5408\u9002\u7684\u767D\u5929\u7D2F\u8BA1\u751F\u957F\uFF0C\u591C\u91CC\u7B49\u5F85\u5929\u4EAE\uFF1B\u77F3\u5B50\u4F1A\u8BA9\u6240\u5728\u683C\u548C\u4E0A\u4E0B\u5DE6\u53F3\u7684\u65E5\u5149\u53EA\u5269 35%\u3002\u6E29\u5EA6\u76EE\u524D\u6CA1\u6709\u5355\u72EC\u6A21\u62DF\u3002\u67E5\u770B\u540E\u786E\u8BA4\uFF1Asoil \u4E2D\u6BCF\u682A\u6240\u5728\u683C\u7684\u6C34\u5206\u548C\u767D\u5929\u5149\u7167\u843D\u5728 requirements \u5BF9\u5E94\u533A\u95F4\uFF1B\u591C\u95F4 care.waitForDaylight \u4E3A true \u65F6\uFF0C\u73AF\u5883\u9002\u5B9C\u5C31\u7B49\u5F85\u5929\u4EAE\u3002", task: "\u7528 terrarium_inspect \u67E5\u770B\u5730\u7BB1\uFF0Csoil \u67E5\u73AF\u5883\u4E0E\u9002\u5B9C\u8303\u56F4\uFF0Ccare \u67E5\u751F\u957F\u6761\u4EF6\u3002", action: "observe" },
  { title: "\u6C34\u58F6\u65C1\u7684\u4FBF\u7B7E", text: "\u65E7\u6C34\u58F6\u65C1\u7559\u7740\u4E00\u884C\u5B57\uFF1A\u201C\u5408\u9002\u6BD4\u66F4\u591A\u91CD\u8981\u3002\u201D\u6D47\u6C34\u9ED8\u8BA4\u662F\u589E\u52A0 35\uFF0Camount \u662F\u52A0\u6C34\u91CF\uFF0CtargetMoisture \u624D\u662F\u8865\u5230\u76EE\u6807\u6C34\u5206\u3002fern \u9002\u5B9C\u6C34\u5206 30\u201385\u3001clover 20\u201380\u3001moss 45\u201390\u3001basil 30\u201375\u3001bean 25\u201370\uFF1Bcare \u4F1A\u7ED9\u51FA\u6BCF\u682A\u9002\u5B9C\u8303\u56F4\u3002\u5148\u6309\u8FD9\u682A\u690D\u7269\u7684\u9002\u5B9C\u533A\u95F4\u9009\u76EE\u6807\uFF1B\u8FC7\u6E7F\u548C\u7F3A\u6C34\u90FD\u4F1A\u6301\u7EED\u635F\u5931\u5065\u5EB7\u3002onlyDry \u4F1A\u8DF3\u8FC7\u5DF2\u8FBE\u5230\u7269\u79CD\u6700\u4F4E\u6C34\u5206\u7684\u683C\u5B50\uFF0C\u4E0D\u662F\u8865\u6EE1\u3002\u6D47\u6C34\u540E\u770B\u56DE\u6267 watering.watered\u3001\u690D\u7269 moistureStatus \u4E0E\u9002\u5B9C\u8303\u56F4\uFF1Bwatered \u4E3A 0 \u8868\u793A\u6CA1\u6709\u52A0\u6C34\uFF0C\u4E0D\u5FC5\u91CD\u590D\u8865\u6EE1\u3002", task: "\u6309\u5B9E\u9645\u6C34\u5206\u9700\u8981\u6D47\u6C34\u4E00\u6B21\uFF1B\u5DF2\u7ECF\u9002\u5B9C\u5C31\u4E0D\u5FC5\u4E3A\u4E86\u505A\u4EFB\u52A1\u53CD\u590D\u6D47\u3002", action: "care" },
  { title: "\u7A7A\u767D\u7684\u690D\u7269\u6807\u7B7E", text: "\u62BD\u5C49\u91CC\u6709\u51E0\u5F20\u7A7A\u767D\u6807\u7B7E\u3002\u6311\u4E00\u683C\u6C34\u5206\u548C\u5149\u7167\u5408\u9002\u7684\u7A7A\u5730\u79CD\u4E0B\u690D\u7269\u3002\u79CD\u5B50\u9700\u8981\u7D2F\u8BA1 2 \u4E2A\u9002\u5B9C\u767D\u5929\u6A21\u62DF\u5C0F\u65F6\u624D\u53D1\u82BD\uFF0C\u591C\u91CC\u4E0D\u7D2F\u8BA1\uFF1B\u540C\u79CD\u53EA\u53D6\u8D21\u732E\u6700\u9AD8\u7684\u524D\u4E09\u682A\uFF0C\u6309 100%\u300150%\u300125% \u8BA1\u5206\uFF0C\u79CD\u5B50\u672C\u8EAB\u4E0D\u8BA1\u5206\u3002\u591A\u6837\u5316\u79CD\u690D\u6BD4\u6324\u6EE1\u540C\u4E00\u79CD\u66F4\u6709\u7528\u3002\u64AD\u79CD\u540E\u770B\u56DE\u6267 seed \u6216 seeds \u7684 ID \u548C\u5750\u6807\u786E\u8BA4\u5165\u571F\uFF1B\u4E4B\u540E\u7528 care \u770B\u53D1\u82BD\u5269\u4F59\u9002\u5B9C\u767D\u5929\u6A21\u62DF\u5C0F\u65F6\uFF0C\u591C\u95F4\u4FDD\u6301\u4E0D\u53D8\u662F\u6B63\u5E38\u7684\u3002", task: "\u5728\u7A7A\u683C\u64AD\u79CD\u4E00\u6B21\uFF1B\u53EF\u7528 terrarium_sow_area \u6279\u91CF\u64AD\u79CD\u3002", action: "plant" },
  { title: "\u4F1A\u52A8\u7684\u90BB\u5C45", text: "\u89C2\u5BDF\u7B14\u8BB0\u8FD8\u7F3A\u51E0\u9875\uFF1A\u8C01\u5403\u4EC0\u4E48\uFF0C\u8C01\u53C8\u9700\u8981\u5E87\u62A4\uFF1F\u5148\u786E\u8BA4\u98DF\u7269\u6765\u6E90\u518D\u653E\u866B\u3002\u869C\u866B\u4F1A\u5543\u690D\u7269\uFF0C\u7532\u866B\u9700\u8981\u869C\u866B\uFF0C\u8718\u86DB\u4F1A\u6355\u98DF\u5176\u4ED6\u866B\uFF1B\u8682\u8681\u7F3A\u7CAE\u4E5F\u53EF\u80FD\u91C7\u96C6\u690D\u7269\u3002\u80FD\u91CF\u4E0E\u9965\u997F\u5206\u5F00\uFF0Cfeed_bug \u53EA\u8865\u80FD\u91CF\uFF0C\u5B9E\u9645\u53D6\u98DF\u624D\u80FD\u964D\u4F4E\u9965\u997F\u3002\u653E\u5165\u8682\u8681\u5E76\u51FA\u73B0\u8681\u5DE2\u540E\uFF0C\u7528 inspect \u7684 colonies \u627E\u5DE2 ID\uFF0C\u518D\u7528 terrarium_feed_nest \u7ED9\u5DE2\u6DFB 5 \u4EFD\u7CAE\u3002\u653E\u866B\u540E\u7528 inspect \u7684 residents \u786E\u8BA4 ID\uFF0C\u518D\u7528 care \u7684 feeding \u770B lastMeal\uFF1B\u9965\u997F\u4E0B\u964D\u624D\u8868\u793A\u5B9E\u9645\u5403\u5230\u4E86\uFF0C\u5355\u8865\u80FD\u91CF\u4E0D\u7B97\u3002", task: "\u786E\u8BA4\u98DF\u7269\u6765\u6E90\u540E\u653E\u5165\u4E00\u53EA\u866B\u5B50\uFF0C\u89C2\u5BDF\u5176\u53D6\u98DF\u4E0E\u9965\u997F\u3002", action: "inhabit" },
  { title: "\u65E7\u4ED3\u5E93\u7684\u94A5\u5319", text: "\u67DC\u5E95\u7684\u94A5\u5319\u7EC8\u4E8E\u627E\u5230\u4E86\u3002\u6536\u85CF\u4F1A\u6682\u505C\u8FD9\u4F4D\u4F4F\u5BA2\u7684\u6A21\u62DF\uFF0C\u4E5F\u4E0D\u518D\u8BA1\u5206\uFF1B\u5176\u4ED6\u751F\u547D\u4ECD\u9700\u7167\u987E\u3002\u653E\u56DE\u690D\u7269\u6216\u79CD\u5B50\u9700\u8981\u4E00\u4E2A\u6CA1\u6709\u690D\u7269\u548C\u79CD\u5B50\u7684\u683C\u5B50\u3002\u9762\u677F\u53EF\u5148\u70B9\u9009\u7A7A\u683C\u518D\u70B9\u201C\u653E\u56DE\u9009\u4E2D\u683C\u201D\uFF0C\u5DE5\u5177\u9700\u8981\u6307\u5B9A x/y\u3002\u6536\u85CF\u540E\u770B\u56DE\u6267 collectionCount \u548C\u6536\u85CF\u5217\u8868\u786E\u8BA4\u6807\u672C\uFF1B\u653E\u56DE\u540E\u67E5\u770B\u56DE\u6267\u5750\u6807\u548C residents\uFF0C\u786E\u8BA4\u5B83\u5DF2\u56DE\u5230\u7BB1\u5185\u3002", task: "\u6536\u85CF\u4E00\u4F4D\u4F4F\u5BA2\uFF1B\u653E\u56DE\u65F6\u5148\u9009\u5408\u6CD5\u4F4D\u7F6E\uFF0C\u4F4D\u7F6E\u4E0D\u5408\u9002\u4E0D\u4F1A\u4E22\u5931\u6536\u85CF\u3002", action: "collect" },
  { title: "\u56F4\u680F\u5916\u7684\u65B0\u5730\u65B9", text: "\u6700\u540E\u4E00\u5C01\u4FE1\u5199\u7740\uFF1A\u201C\u5F53\u8FD9\u91CC\u8DB3\u591F\u7E41\u76DB\uFF0C\u56F4\u680F\u5916\u8FD8\u6709\u5730\u65B9\u3002\u201D\u6269\u5EFA\u6309\u65B0\u516C\u5F0F\u5DF2\u8BB0\u5F55\u7684\u6700\u9AD8\u5206\u5224\u5B9A\uFF0C\u4E0D\u82B1\u5206\uFF0C\u4E5F\u4E0D\u4F1A\u56E0\u5F53\u524D\u5206\u4E0B\u964D\u7F29\u5C0F\u3002\u5730\u7BB1\u95E8\u69DB\u4E3A 120\u3001260\u3001420\uFF1B\u65E7\u516C\u5F0F\u9AD8\u5206\u4E0D\u7528\u4E8E\u89E3\u9501\u3002\u540C\u79CD\u53EA\u8BA1\u524D\u4E09\uFF0C\u52A8\u7269\u517B\u6EE1 24 \u4E2A\u6A21\u62DF\u5C0F\u65F6\u624D\u6709\u5B8C\u6574\u6210\u719F\u5EA6\u8D21\u732E\uFF0C\u8FC7\u997F\u4F1A\u964D\u5206\u3002\u6269\u5EFA\u540E\u7528 terrarium_progress \u786E\u8BA4 level\u3001size \u548C collection \u5BB9\u91CF\u5DF2\u589E\u52A0\uFF1B\u6700\u9AD8\u5206\u4FDD\u7559\uFF0C\u4E0D\u6263\u5206\u3002", task: "\u65B0\u516C\u5F0F\u6700\u9AD8\u5206\u8FBE\u5230 120 \u540E\u6269\u5EFA\u5230\u4E8C\u7EA7\uFF1B\u5267\u60C5\u6CA1\u6709\u989D\u5916\u79EF\u5206\u5956\u52B1\u3002", action: "upgrade" }
];
function validDevelopment(value, habitat) {
  const p = value.progression;
  if (p !== void 0 && (!p || p.scoreVersion !== void 0 && p.scoreVersion !== 1 && p.scoreVersion !== TERRARIUM_SCORE_VERSION || !TERRARIUM_LEVELS.some((l) => l.level === p.level && Number.isSafeInteger(p.peakScore) && p.peakScore >= 0) || !Number.isInteger(p.guideStep) || p.guideStep < 0 || p.guideStep > LAND_LETTERS.length || typeof p.guideSkipped !== "boolean" || !Array.isArray(p.actions) || p.actions.length > ACTIONS.length || new Set(p.actions).size !== p.actions.length || !p.actions.every((a) => ACTIONS.includes(a)))) return false;
  return value.facilities === void 0 || Array.isArray(value.facilities) && value.facilities.length <= Object.values(TERRARIUM_FACILITIES).filter((f) => f.habitat === habitat).length && new Set(value.facilities.map((f) => f?.kind)).size === value.facilities.length && value.facilities.every((f) => f && Object.hasOwn(TERRARIUM_FACILITIES, f.kind) && TERRARIUM_FACILITIES[f.kind].habitat === habitat && capacity(value).level >= TERRARIUM_FACILITIES[f.kind].level && inHabitat(value, f.x, f.y) && typeof f.enabled === "boolean");
}

// ../terrarium-server/bridge/legacy-src/colony.ts
var TERRARIUM_NEST_SIZE = 4;
var TERRARIUM_NEST_MAX_SIZE = 8;
var TERRARIUM_NEST_DIG_ACTIONS = 12;
var nestSize = (nest) => nest.size ?? TERRARIUM_NEST_SIZE;
var nestEntrances = (nest) => [{ x: nest.x, y: nest.y }, ...nest.entrances ?? []];
function validColony(state) {
  const nests = state.nests ?? [];
  const validResident = (resident) => {
    if (resident.inside === void 0) return true;
    const p = resident.inside;
    const nest = nests.find((nest2) => nest2.id === p?.nestId);
    return resident.species === "ant" && !!nest && resident.x === nest.x && resident.y === nest.y && Number.isInteger(p.x) && p.x >= 0 && p.x < nestSize(nest) && Number.isInteger(p.y) && p.y >= 0 && p.y < nestSize(nest);
  };
  if (!nests.every((n) => (n.size === void 0 || Number.isInteger(n.size) && n.size >= TERRARIUM_NEST_SIZE && n.size <= TERRARIUM_NEST_MAX_SIZE) && (n.digProgress === void 0 || Number.isInteger(n.digProgress) && n.digProgress >= 0 && n.digProgress < TERRARIUM_NEST_DIG_ACTIONS) && (n.entrances === void 0 || Array.isArray(n.entrances) && n.entrances.length <= nestSize(n) - 4 && n.entrances.every((p) => p && inHabitat(state, p.x, p.y))))) return false;
  const entrances = nests.flatMap(nestEntrances);
  if (new Set(entrances.map((p) => `${p.x},${p.y}`)).size !== entrances.length) return false;
  return state.bugs.every((bug) => (bug.role === void 0 || bug.species === "ant" && ["queen", "worker"].includes(bug.role)) && validResident(bug)) && state.bugs.filter((bug) => bug.role === "queen").length <= 1 && (state.eggs ?? []).every(validResident);
}

// ../terrarium-server/bridge/legacy-src/merchant.ts
var TERRARIUM_MAX_COINS = 1e9;
var TERRARIUM_POTS = {
  clay: { name: "\u7D20\u9676\u76C6", price: 24, effect: "\u6696\u9676\u8272\u88C5\u9970\uFF0C\u65E0\u989D\u5916\u6548\u679C", growth: 1, evaporation: 1, light: 0, recovery: 0, drain: 0 },
  porcelain: { name: "\u9752\u74F7\u76C6", price: 40, effect: "\u9752\u74F7\u4E0E\u7EC6\u53F6\u7EB9\u88C5\u9970\uFF0C\u65E0\u989D\u5916\u6548\u679C", growth: 1, evaporation: 1, light: 0, recovery: 0, drain: 0 },
  spring: { name: "\u6625\u9192\u76C6", price: 100, effect: "\u81EA\u7136\u751F\u957F \xD71.5\uFF0C\u84B8\u53D1 \xD71.3", growth: 1.5, evaporation: 1.3, light: 0, recovery: 0, drain: 0 },
  slow: { name: "\u7F13\u65F6\u76C6", price: 60, effect: "\u81EA\u7136\u751F\u957F \xD70.5\uFF0C\u84B8\u53D1 \xD70.8", growth: 0.5, evaporation: 0.8, light: 0, recovery: 0, drain: 0 },
  still: { name: "\u7559\u666F\u76C6", price: 120, effect: "\u505C\u6B62\u840C\u53D1\u548C\u751F\u957F\uFF0C\u7981\u6B62\u65BD\u80A5\uFF1B\u6C34\u5206\u3001\u5065\u5EB7\u548C\u91C7\u98DF\u4ECD\u6B63\u5E38\u7ED3\u7B97", growth: 0, evaporation: 1, light: 0, recovery: 0, drain: 0 },
  reservoir: { name: "\u84C4\u9732\u76C6", price: 80, effect: "\u84B8\u53D1 \xD70.45\uFF1B\u4E0D\u81EA\u52A8\u6D47\u6C34", growth: 1, evaporation: 0.45, light: 0, recovery: 0, drain: 0 },
  drainage: { name: "\u900F\u96E8\u76C6", price: 60, effect: "\u6BCF\u5C0F\u65F6\u989D\u5916\u6392\u51FA 2 \u6C34\u5206\uFF0C\u6700\u4F4E\u6392\u5230 60\uFF1B\u4ECD\u4F1A\u81EA\u7136\u84B8\u53D1", growth: 1, evaporation: 1, light: 0, recovery: 0, drain: 2 },
  shade: { name: "\u82D4\u836B\u76C6", price: 80, effect: "\u672C\u683C\u767D\u5929\u5149\u7167 \xD70.55\uFF0C\u84B8\u53D1 \xD70.8", growth: 1, evaporation: 0.8, light: -0.45, recovery: 0, drain: 0 },
  sun: { name: "\u6620\u65E5\u76C6", price: 120, effect: "\u672C\u683C\u767D\u5929\u5149\u7167 +0.25\uFF0C\u4E0A\u9650 1\uFF1B\u4E0D\u6539\u53D8\u663C\u591C", growth: 1, evaporation: 1, light: 0.25, recovery: 0, drain: 0 },
  healing: { name: "\u517B\u53F6\u76C6", price: 160, effect: "\u9002\u5B9C\u767D\u5929\u6BCF\u5C0F\u65F6\u6062\u590D 1 \u5065\u5EB7\u53CA 2 \u53F6\u635F\uFF1B\u81EA\u7136\u751F\u957F \xD70.8", growth: 0.8, evaporation: 1, light: 0, recovery: 1, drain: 0 }
};
function validEconomy(state) {
  const e = state.economy;
  if (e === void 0) return true;
  if (!e || !Number.isSafeInteger(e.earned) || e.earned < 0 || e.earned > TERRARIUM_MAX_COINS || !Number.isSafeInteger(e.spent) || e.spent < 0 || e.spent > e.earned || !e.inventory || typeof e.inventory !== "object" || Array.isArray(e.inventory) || !Array.isArray(e.pots) || e.pots.length > 64) return false;
  let owned = e.pots.length, cost = 0;
  for (const [kind, count] of Object.entries(e.inventory)) {
    if (!Object.hasOwn(TERRARIUM_POTS, kind) || !Number.isSafeInteger(count) || count < 0 || count > 64) return false;
    owned += count;
    cost += TERRARIUM_POTS[kind].price * count;
  }
  for (const p of e.pots) {
    if (!p || !Object.hasOwn(TERRARIUM_POTS, p.kind) || !inHabitat(state, p.x, p.y)) return false;
    cost += TERRARIUM_POTS[p.kind].price;
  }
  return owned <= 64 && cost === e.spent && new Set(e.pots.map((p) => `${p.x},${p.y}`)).size === e.pots.length;
}
var TERRARIUM_MERCHANT_TOOL_DECL = {
  name: "terrarium_merchant",
  tags: ["read", "write"],
  description: "Land merchant and pot shop. status returns coins, inventory, catalog and plant sale quotes. sell permanently removes a living land plant by id: requires 6 natural growth hours and health >=40; fertilizer grants no sale value. buy purchases one pot into inventory; place equips it at x,y (replaced pot returns to inventory); remove returns a placed pot. Pots stay when plants are sold, composted, collected or die. One effect per plot; no stacking. still blocks fertilizer but does not freeze health, water or insects. Coins are local snapshot data, separate from ecology score; decorative coin item is unrelated.",
  parameters: { type: "object", additionalProperties: false, properties: { action: { type: "string", enum: ["status", "sell", "buy", "place", "remove"] }, id: { type: "string" }, pot: { type: "string", enum: Object.keys(TERRARIUM_POTS) }, x: { type: "integer", minimum: 0 }, y: { type: "integer", minimum: 0 } } }
};

// ../terrarium-server/bridge/legacy-src/habitat.ts
var TERRARIUM_PLANT_REQUIREMENTS = {
  fern: { minMoisture: 30, maxMoisture: 85, minLight: 0.15, maxLight: 0.7 },
  clover: { minMoisture: 20, maxMoisture: 80, minLight: 0.45, maxLight: 1 },
  moss: { minMoisture: 45, maxMoisture: 90, minLight: 0.15, maxLight: 0.4 },
  basil: { minMoisture: 30, maxMoisture: 75, minLight: 0.65, maxLight: 1 },
  bean: { minMoisture: 25, maxMoisture: 70, minLight: 0.7, maxLight: 1 },
  sunflower: { minMoisture: 25, maxMoisture: 70, minLight: 0.75, maxLight: 1 },
  daisy: { minMoisture: 25, maxMoisture: 75, minLight: 0.6, maxLight: 1 },
  lavender: { minMoisture: 20, maxMoisture: 60, minLight: 0.7, maxLight: 1 },
  mint: { minMoisture: 35, maxMoisture: 85, minLight: 0.3, maxLight: 0.85 }
};
var TERRARIUM_MAX_HISTORY = 168;
var TERRARIUM_SUNFLOWER = { seedHours: 30, seedIntervalHours: 12, seedPortions: 3 };
function sunflowerSeedRounds(growthHours) {
  return Math.max(0, Math.floor((growthHours - TERRARIUM_SUNFLOWER.seedHours) / TERRARIUM_SUNFLOWER.seedIntervalHours) + 1);
}

// ../terrarium-server/bridge/legacy-src/pollination.ts
var TERRARIUM_POLLINATION = { nectarIntervalMs: 30 * 6e4, maxCharges: 3, bonusSeedPortions: 2 };
function isPollinator(species) {
  return species === "bee" || species === "butterfly";
}
function producesSeeds(species) {
  return ["sunflower", "daisy", "lavender"].includes(species);
}

// ../terrarium-server/bridge/legacy-src/fish-breeding.ts
var TERRARIUM_FISH_BREEDING = { maturityHours: 24, juvenileHours: 12, intervalHours: 12, incubationHours: 4, eggsPerClutch: 2 };
var TERRARIUM_SPAWNING_SPECIES = ["neon_tetra", "corydoras"];

// ../terrarium-server/bridge/legacy-src/care-alerts.ts
function validCareAlerts(value) {
  return value === void 0 || !!value && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length <= 100 && Object.entries(value).every(([key, v]) => key.length <= 100 && v && [1, 2].includes(v.severity) && typeof v.label === "string" && v.label.length <= 200);
}

// ../terrarium-server/bridge/legacy-src/deaths.ts
var TERRARIUM_MAX_DEATH_RECORDS = 48;
function validDeaths(value, atMs, gridSize = 5) {
  if (value === void 0) return true;
  return Array.isArray(value) && value.length <= TERRARIUM_MAX_DEATH_RECORDS && value.every((r, i) => {
    if (!r || !Number.isFinite(r.recordedAtMs) || r.recordedAtMs > atMs || i > 0 && r.recordedAtMs < value[i - 1].recordedAtMs || typeof r.cause !== "string" || r.cause.length > 1e3) return false;
    const o = r.organism;
    return o && typeof o.id === "string" && o.id.length <= 32 && typeof o.species === "string" && o.species.length <= 32 && [o.x, o.y].every((n) => Number.isInteger(n) && n >= 0 && n < gridSize) && ["energy", "hunger", "health", "ageHours"].every((k) => o[k] === void 0 || Number.isFinite(o[k]) && o[k] >= 0) && validMeal(o.lastMeal, r.recordedAtMs) && r.environment && typeof r.environment === "object" && !Array.isArray(r.environment) && Object.keys(r.environment).length <= 10 && Object.values(r.environment).every((v) => typeof v === "number" ? Number.isFinite(v) : typeof v === "string" && v.length <= 100);
  });
}
function validMeal(value, atMs) {
  if (value === void 0) return true;
  if (!value || typeof value !== "object") return false;
  const meal = value;
  return Number.isFinite(meal.atMs) && meal.atMs <= atMs && typeof meal.kind === "string" && meal.kind.length <= 32 && (meal.id === void 0 || typeof meal.id === "string" && meal.id.length <= 32);
}

// ../terrarium-server/bridge/legacy-src/replay.ts
var TERRARIUM_MAX_REPLAYS = 16;
var TERRARIUM_REPLAY_RETENTION_MS = 7 * 24 * 60 * 6e4;

// ../terrarium-server/bridge/legacy-src/aquatic-habitat.ts
var TERRARIUM_FISH_SPECIES = ["guppy", "goldfish", "perch", "neon_tetra", "corydoras"];
var TERRARIUM_AQUATIC_PLANTS = {
  waterweed: { dayOxygen: 0.8, nightOxygen: -0.2, purification: 0 },
  waterlily: { dayOxygen: 0.5, nightOxygen: -0.1, purification: 0 },
  algae: { dayOxygen: 1.1, nightOxygen: -0.3, purification: 0 },
  hornwort: { dayOxygen: 1.2, nightOxygen: -0.25, purification: 0 },
  duckweed: { dayOxygen: 0.6, nightOxygen: -0.15, purification: 0.12 }
};

// ../terrarium-server/bridge/legacy-src/aquarium.ts
var HOUR_MS = 36e5;
var TERRARIUM_FISH_ACTION_INTERVAL_MS = 10 * 6e4;
var percent = (value) => Number.isFinite(value) && value >= 0 && value <= 100;
function newAquarium(nowMs) {
  const state = { version: 1, kind: "water", updatedAtMs: nowMs, nextId: 1, oxygen: 80, cleanliness: 100, fish: [], plants: [], food: [], history: [], replays: [] };
  plantAquatic(state, "waterweed", 0, 0);
  plantAquatic(state, "algae", 4, 0);
  delete state.progression;
  recordHistory(state, nowMs);
  return state;
}
function plantAquatic(state, species, x, y) {
  if (typeof species !== "string" || !Object.hasOwn(TERRARIUM_AQUATIC_PLANTS, species)) throw new Error("Unknown aquatic plant species");
  if (!inHabitat(state, x, y)) throw new Error("Position must be in the current habitat grid");
  if (state.plants.length >= capacity(state).organisms) throw new Error("Aquatic plant capacity reached");
  if (state.plants.some((plant2) => plant2.x === x && plant2.y === y)) throw new Error("That plot already has an aquatic plant");
  const plant = { id: `aquatic-${state.nextId++}`, species, x, y, health: 100, growthHours: 0, stage: 0 };
  state.plants.push(plant);
  recordAction(state, "plant");
  return plant;
}
function recordHistory(state, recordedAtMs) {
  if (state.progression) observeProgress(state);
  state.history.push({ ...state.progression ? { progression: structuredClone(state.progression) } : {}, recordedAtMs, score: score(state), scoreVersion: TERRARIUM_SCORE_VERSION, oxygen: state.oxygen, cleanliness: state.cleanliness, fish: structuredClone(state.fish) });
  state.history = state.history.filter((sample) => sample.recordedAtMs >= recordedAtMs - 168 * HOUR_MS).slice(-168);
}
function validFrame(value, atMs) {
  if (!value || typeof value !== "object") return false;
  const s = value;
  const id = (value2) => typeof value2 === "string" && value2.length >= 1 && value2.length <= 32;
  return validDevelopment(s, "water") && s.kind === "water" && ["day", "night"].includes(s.phase) && percent(s.oxygen) && percent(s.cleanliness) && Array.isArray(s.fish) && s.fish.length <= capacity(s).organisms && s.fish.every((f) => f && id(f.id) && TERRARIUM_FISH_SPECIES.includes(f.species) && inHabitat(s, f.x, f.y) && percent(f.energy) && percent(f.health) && Number.isFinite(f.ageHours) && f.ageHours >= 0 && (f.juvenile === void 0 || typeof f.juvenile === "boolean") && (f.lastSpawnAgeHours === void 0 || Number.isFinite(f.lastSpawnAgeHours) && f.lastSpawnAgeHours >= 0 && f.lastSpawnAgeHours <= f.ageHours) && (f.lastMeal === void 0 || f.lastMeal && Number.isFinite(f.lastMeal.atMs) && f.lastMeal.atMs <= atMs && ["fish", "food", "algae"].includes(f.lastMeal.kind) && id(f.lastMeal.id))) && (s.eggs === void 0 || Array.isArray(s.eggs) && s.eggs.length <= capacity(s).organisms && s.eggs.every((e) => e && id(e.id) && TERRARIUM_SPAWNING_SPECIES.includes(e.species) && inHabitat(s, e.x, e.y) && Number.isFinite(e.laidAtMs) && e.laidAtMs <= atMs && Number.isFinite(e.incubationHours) && e.incubationHours >= 0 && e.incubationHours <= TERRARIUM_FISH_BREEDING.incubationHours && percent(e.health))) && s.fish.length + (s.eggs?.length ?? 0) <= capacity(s).organisms && Array.isArray(s.plants) && s.plants.length <= capacity(s).organisms && s.plants.every((p) => p && id(p.id) && Object.hasOwn(TERRARIUM_AQUATIC_PLANTS, p.species) && inHabitat(s, p.x, p.y) && percent(p.health) && Number.isFinite(p.growthHours) && p.growthHours >= 0 && Number.isInteger(p.stage) && p.stage >= 0 && p.stage <= 3) && Array.isArray(s.food) && s.food.length <= 20 && s.food.every((f) => f && id(f.id) && inHabitat(s, f.x, f.y) && Number.isInteger(f.portions) && f.portions >= 1 && f.portions <= 10 && Number.isFinite(f.droppedAtMs) && f.droppedAtMs <= atMs) && new Set([...s.fish, ...s.plants, ...s.food, ...s.eggs ?? []].map((v) => v.id)).size === s.fish.length + s.plants.length + s.food.length + (s.eggs?.length ?? 0) && new Set(s.plants.map((p) => `${p.x},${p.y}`)).size === s.plants.length;
}
function validAquarium(value) {
  if (!value || typeof value !== "object") return false;
  const s = value;
  return validCareAlerts(s.careAlerts) && validDeaths(s.deaths, s.updatedAtMs, capacity(s).size) && validCollection(s.collection, "water", capacity(s).collection) && (s.oxygenWarning === void 0 || typeof s.oxygenWarning === "boolean") && s.version === 1 && Number.isFinite(s.updatedAtMs) && Number.isSafeInteger(s.nextId) && s.nextId >= 1 && (s.name === void 0 || typeof s.name === "string" && s.name === s.name.trim() && Array.from(s.name).length >= 1 && Array.from(s.name).length <= TERRARIUM_MAX_NAME_LENGTH && !/[\u0000-\u001f\u007f-\u009f]/.test(s.name)) && validFrame({ ...s, phase: "day" }, s.updatedAtMs) && new Set([...s.fish, ...s.plants, ...s.food, ...s.eggs ?? [], ...(s.collection ?? []).map((e) => e.specimen)].map((e) => e.id)).size === s.fish.length + s.plants.length + s.food.length + (s.eggs?.length ?? 0) + (s.collection?.length ?? 0) && Array.isArray(s.history) && s.history.length <= 168 && s.history.every((h, i) => h && Number.isFinite(h.recordedAtMs) && (h.scoreVersion === void 0 || h.scoreVersion === 1 || h.scoreVersion === TERRARIUM_SCORE_VERSION) && (h.score === void 0 || Number.isSafeInteger(h.score) && h.score >= 0) && h.recordedAtMs <= s.updatedAtMs && (i === 0 || h.recordedAtMs > s.history[i - 1].recordedAtMs) && percent(h.oxygen) && percent(h.cleanliness) && validFrame({ progression: h.progression, kind: "water", phase: "day", oxygen: h.oxygen, cleanliness: h.cleanliness, fish: h.fish, plants: [], food: [] }, h.recordedAtMs)) && Array.isArray(s.replays) && s.replays.length <= TERRARIUM_MAX_REPLAYS && new Set(s.replays.map((r) => r?.id)).size === s.replays.length && s.replays.every((r, i) => {
    if (!r || typeof r.id !== "string" || r.id.length > 160 || !Number.isFinite(r.recordedAtMs) || r.recordedAtMs > s.updatedAtMs || i > 0 && r.recordedAtMs < s.replays[i - 1].recordedAtMs || !Array.isArray(r.frames) || r.frames.length !== 3 || !r.frames.every((f) => validFrame(f, r.recordedAtMs))) return false;
    const [before, contact, after] = r.frames;
    const hunter = contact.fish.find((f) => f.id === r.hunterId), prey = contact.fish.find((f) => f.id === r.preyId);
    return !!hunter && !!prey && hunter.species === "perch" && (prey.species === "guppy" || prey.species === "neon_tetra") && hunter.x === prey.x && hunter.y === prey.y && before.fish.some((f) => f.id === hunter.id) && before.fish.some((f) => f.id === prey.id) && after.fish.some((f) => f.id === hunter.id) && !after.fish.some((f) => f.id === prey.id);
  });
}

// ../terrarium-server/bridge/legacy-src/collection.ts
function validCollection(value, habitat, limit = 48) {
  if (value === void 0) return true;
  if (!Array.isArray(value) || value.length > limit || new Set(value.map((e) => e?.specimen?.id)).size !== value.length) return false;
  const specimenProgress = { level: 4, peakScore: 420, guideStep: 0, guideSkipped: false, actions: [] };
  return value.every((entry) => {
    if (!entry || !entry.specimen) return false;
    if (habitat === "water") {
      const base2 = newAquarium(0);
      base2.progression = specimenProgress;
      base2.plants = [];
      base2.fish = [];
      if (entry.kind === "fish") {
        base2.fish = [entry.specimen];
        base2.updatedAtMs = entry.specimen.lastMeal?.atMs ?? 0;
      } else if (entry.kind === "aquatic") base2.plants = [entry.specimen];
      else return false;
      return validAquarium(base2);
    }
    const base = newState(0);
    base.progression = specimenProgress;
    base.soil = Array.from({ length: 64 }, (_, i) => ({ x: i % 8, y: Math.floor(i / 8), moisture: 50 }));
    base.history = [];
    base.plants = [];
    base.bugs = [];
    base.seeds = [];
    if (entry.kind === "plant") base.plants = [entry.specimen];
    else if (entry.kind === "seed") base.seeds = [entry.specimen];
    else if (entry.kind === "bug") {
      base.bugs = [entry.specimen];
      base.updatedAtMs = entry.specimen.lastMeal?.atMs ?? 0;
    } else return false;
    return validState(base);
  });
}

// ../terrarium-server/bridge/legacy-src/flight.ts
var TERRARIUM_FLY_SPECIES = ["fruit_fly", "mosquito", "moth", "bee", "butterfly"];
function isFlying(species) {
  return TERRARIUM_FLY_SPECIES.includes(species);
}

// ../terrarium-server/bridge/legacy-src/state.ts
var TERRARIUM_MAX_NAME_LENGTH = 64;
function validName(value) {
  return typeof value === "string" && value === value.trim() && Array.from(value).length >= 1 && Array.from(value).length <= TERRARIUM_MAX_NAME_LENGTH && !/[\u0000-\u001f\u007f-\u009f]/.test(value);
}
var TERRARIUM_WEB_LIFETIME_MS = 6 * 60 * 6e4;
var TERRARIUM_WEBS_PER_SPIDER = 3;
var HOUR_MS2 = 60 * 6e4;
var TERRARIUM_MAX_CATCHUP_MS = 30 * 24 * HOUR_MS2;
function newState(nowMs) {
  const state = { version: 3, soil: Array.from({ length: 25 }, (_, i) => ({ x: i % 5, y: Math.floor(i / 5), moisture: 50 })), history: [], updatedAtMs: nowMs, nextId: 1, plants: [
    { id: "fern", species: "fern", x: 0, y: 0, moisture: 45, growthHours: 0, naturalGrowthHours: 0, stage: 0, dry: false, health: 100 },
    { id: "clover", species: "clover", x: 4, y: 0, moisture: 50, growthHours: 0, naturalGrowthHours: 0, stage: 0, dry: false, health: 100 }
  ], seeds: [], bugs: [], items: [{ id: "coin", x: 1, y: 1 }, { id: "pebble", x: 2, y: 1 }] };
  for (const plant of state.plants) state.soil[plant.y * capacity(state).size + plant.x].moisture = plant.moisture;
  recordHistory2(state, nowMs);
  return state;
}
function recordHistory2(state, recordedAtMs) {
  if (state.progression) observeProgress(state);
  state.history.push({ recordedAtMs, gridSize: capacity(state).size, score: score(state), scoreVersion: TERRARIUM_SCORE_VERSION, soilMoisture: state.soil.map((cell) => cell.moisture), plants: state.plants.map(({ id, species, moisture, health, stage, growthHours }) => ({ id, species, moisture, health, stage, growthHours })) });
  state.history = state.history.filter((sample) => sample.recordedAtMs >= recordedAtMs - TERRARIUM_MAX_HISTORY * HOUR_MS2).slice(-TERRARIUM_MAX_HISTORY);
}
function migrateState(value) {
  if (value?.version === 1) {
    const old = value;
    if (!Number.isFinite(old.updatedAtMs) || !Array.isArray(old.plants) || old.plants.length !== 2 || !Array.isArray(old.items)) throw new Error("Invalid terrarium state file");
    const migrated = newState(old.updatedAtMs);
    migrated.plants = old.plants.map((p) => ({ ...p, species: p.id, x: p.id === "fern" ? 0 : 4, y: 0, health: 100 }));
    migrated.items = old.items;
    if (!validState(migrated)) throw new Error("Invalid terrarium state file");
    initializeHabitat(migrated);
    return migrated;
  }
  if (value?.version === 2) {
    const migrated = { ...value, version: 3, soil: newState(0).soil, history: [] };
    if (!validState(migrated)) throw new Error("Invalid terrarium state file");
    initializeHabitat(migrated);
    return migrated;
  }
  if (value?.version === 3 && Array.isArray(value.history)) {
    value = { ...value, history: value.history.map((sample) => {
      if (sample && typeof sample === "object" && !Object.hasOwn(sample, "recordedAtMs") && Object.hasOwn(sample, "atMs")) {
        const { atMs, ...rest } = sample;
        return { ...rest, recordedAtMs: atMs };
      }
      return sample;
    }) };
  }
  if (!validState(value)) throw new Error("Invalid terrarium state file");
  return value;
}
function initializeHabitat(state) {
  for (const plant of state.plants) {
    state.soil[plant.y * capacity(state).size + plant.x].moisture = plant.moisture;
    plant.dry = plant.moisture < TERRARIUM_PLANT_REQUIREMENTS[plant.species].minMoisture;
  }
  state.history = [];
  recordHistory2(state, state.updatedAtMs);
}
function validHabitat(state) {
  const percent2 = (value) => Number.isFinite(value) && value >= 0 && value <= 100;
  return Array.isArray(state.soil) && state.soil.length === capacity(state).size ** 2 && state.soil.every((cell, i) => cell && cell.x === i % capacity(state).size && cell.y === Math.floor(i / capacity(state).size) && percent2(cell.moisture) && (cell.allowAntForaging === void 0 || typeof cell.allowAntForaging === "boolean") && (cell.antBackfilled === void 0 || typeof cell.antBackfilled === "boolean")) && Array.isArray(state.history) && state.history.length <= TERRARIUM_MAX_HISTORY && state.history.every((sample, i) => sample && Number.isFinite(sample.recordedAtMs) && (sample.scoreVersion === void 0 || sample.scoreVersion === 1 || sample.scoreVersion === TERRARIUM_SCORE_VERSION) && (sample.score === void 0 || Number.isSafeInteger(sample.score) && sample.score >= 0) && sample.recordedAtMs <= state.updatedAtMs && (i === 0 || sample.recordedAtMs > state.history[i - 1].recordedAtMs) && Array.isArray(sample.soilMoisture) && [5, 6, 7, 8].includes(sample.gridSize ?? 5) && (sample.gridSize ?? 5) <= capacity(state).size && sample.soilMoisture.length === (sample.gridSize ?? 5) ** 2 && sample.soilMoisture.every(percent2) && Array.isArray(sample.plants) && sample.plants.length <= (TERRARIUM_LEVELS.find((l) => l.size === (sample.gridSize ?? 5))?.organisms ?? 12) && sample.plants.every((p) => p && typeof p.id === "string" && p.id.length <= 32 && Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS, p.species) && percent2(p.moisture) && percent2(p.health) && Number.isInteger(p.stage) && p.stage >= 0 && p.stage <= 3 && Number.isFinite(p.growthHours) && p.growthHours >= 0));
}
function validReplays(state) {
  return state.replays === void 0 || Array.isArray(state.replays) && state.replays.length <= TERRARIUM_MAX_REPLAYS && state.replays.every((replay, i) => {
    if (!replay || typeof replay.id !== "string" || replay.id.length > 160 || typeof replay.hunterId !== "string" || typeof replay.preyId !== "string" || !Number.isFinite(replay.recordedAtMs) || replay.recordedAtMs > state.updatedAtMs || i > 0 && replay.recordedAtMs < state.replays[i - 1].recordedAtMs || !Array.isArray(replay.frames) || replay.frames.length !== 3) return false;
    if (!replay.frames.every((frame) => frame && (frame.phase === "day" || frame.phase === "night") && validState({ ...frame, version: 3, nextId: state.nextId, updatedAtMs: replay.recordedAtMs, history: [], replays: void 0 }))) return false;
    const [before, contact, after] = replay.frames;
    const hunter = contact.bugs.find((bug) => bug.id === replay.hunterId);
    const prey = contact.bugs.find((bug) => bug.id === replay.preyId);
    return !!hunter && !!prey && hunter.x === prey.x && hunter.y === prey.y && (hunter.species === "spider" ? prey.species !== "spider" : ["beetle", "ladybug"].includes(hunter.species) && prey.species === "aphid") && before.bugs.some((bug) => bug.id === hunter.id) && before.bugs.some((bug) => bug.id === prey.id) && after.bugs.some((bug) => bug.id === hunter.id) && !after.bugs.some((bug) => bug.id === prey.id);
  }) && new Set(state.replays.map((replay) => replay.id)).size === state.replays.length;
}
function validState(value) {
  if (!value || typeof value !== "object") return false;
  const state = value;
  if (!validEconomy(state) || state.fertilizer !== void 0 && (!Number.isSafeInteger(state.fertilizer) || state.fertilizer < 0) || !validDevelopment(state, "land") || !validCareAlerts(state.careAlerts) || !validDeaths(state.deaths, state.updatedAtMs, capacity(state).size) || !validCollection(state.collection, "land", capacity(state).collection) || state.irrigation !== void 0 && (!Array.isArray(state.irrigation) || state.irrigation.length > capacity(state).size ** 2 || new Set(state.irrigation.map((c) => `${c?.x},${c?.y}`)).size !== state.irrigation.length || !state.irrigation.every((c) => c && inHabitat(state, c.x, c.y) && Number.isFinite(c.targetMoisture) && c.targetMoisture >= 0 && c.targetMoisture <= 100 && Number.isFinite(c.intervalMinutes) && c.intervalMinutes >= 1 && c.intervalMinutes <= 1440 && Number.isFinite(c.lastWateredAtMs)))) return false;
  return state.version === 3 && (state.name === void 0 || validName(state.name)) && validHabitat(state) && validReplays(state) && Number.isFinite(state.updatedAtMs) && Number.isSafeInteger(state.nextId) && state.nextId >= 1 && Array.isArray(state.plants) && state.plants.length <= capacity(state).organisms && state.plants.every((p) => p && typeof p.id === "string" && p.id.length <= 32 && Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS, p.species) && inHabitat(state, p.x, p.y) && Number.isFinite(p.moisture) && p.moisture >= 0 && p.moisture <= 100 && Number.isFinite(p.growthHours) && p.growthHours >= 0 && (p.naturalGrowthHours === void 0 || Number.isFinite(p.naturalGrowthHours) && p.naturalGrowthHours >= 0 && p.naturalGrowthHours <= 48 && p.naturalGrowthHours <= p.growthHours + 1e-8) && Number.isInteger(p.stage) && p.stage >= 0 && p.stage <= 3 && typeof p.dry === "boolean" && Number.isFinite(p.health) && p.health >= 0 && p.health <= 100 && (p.pollination === void 0 || Number.isSafeInteger(p.pollination) && p.pollination >= 0 && p.pollination <= TERRARIUM_POLLINATION.maxCharges) && (p.healthWarning === void 0 || typeof p.healthWarning === "boolean") && (p.leafDamage === void 0 || Number.isFinite(p.leafDamage) && p.leafDamage >= 0 && p.leafDamage <= 100) && (p.seedRounds === void 0 || producesSeeds(p.species) && Number.isSafeInteger(p.seedRounds) && p.seedRounds >= 0 && p.seedRounds <= sunflowerSeedRounds(p.growthHours))) && Array.isArray(state.seeds) && state.seeds.length <= capacity(state).organisms && state.plants.length + state.seeds.length <= capacity(state).organisms && state.seeds.every((s) => s && typeof s.id === "string" && Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS, s.species) && inHabitat(state, s.x, s.y) && Number.isFinite(s.ageHours) && s.ageHours >= 0) && Array.isArray(state.bugs) && state.bugs.length <= capacity(state).organisms && state.bugs.every((b) => b && typeof b.id === "string" && (["aphid", "beetle", "ladybug", "spider", "ant"].includes(b.species) || isFlying(b.species)) && inHabitat(state, b.x, b.y) && Number.isFinite(b.energy) && b.energy >= 0 && b.energy <= 100 && (b.pollen === void 0 || isPollinator(b.species) && b.pollen && typeof b.pollen.plantId === "string" && b.pollen.plantId.length >= 1 && b.pollen.plantId.length <= 32 && Object.hasOwn(TERRARIUM_PLANT_REQUIREMENTS, b.pollen.species)) && validMeal(b.lastMeal, state.updatedAtMs) && (b.hunger === void 0 || Number.isFinite(b.hunger) && b.hunger >= 0 && b.hunger <= 100) && (b.carrying === void 0 || b.species === "ant" && Number.isInteger(b.carrying) && b.carrying >= 0 && b.carrying <= 5) && Number.isFinite(b.ageHours) && b.ageHours >= 0) && (state.eggs === void 0 || Array.isArray(state.eggs) && state.eggs.length <= capacity(state).organisms && state.eggs.every((egg) => egg && typeof egg.id === "string" && egg.id.length <= 32 && (["beetle", "ladybug", "spider", "ant"].includes(egg.species) || isFlying(egg.species)) && inHabitat(state, egg.x, egg.y) && Number.isFinite(egg.laidAtMs) && egg.laidAtMs <= state.updatedAtMs && Number.isFinite(egg.hatchesAtMs) && egg.hatchesAtMs > egg.laidAtMs && egg.hatchesAtMs <= egg.laidAtMs + 6 * HOUR_MS2)) && state.bugs.length + (state.eggs?.length ?? 0) <= capacity(state).organisms && (state.nests === void 0 || Array.isArray(state.nests) && state.nests.length <= 5 && state.nests.every((nest) => nest && typeof nest.id === "string" && nest.id.length <= 32 && inHabitat(state, nest.x, nest.y) && Number.isInteger(nest.depth) && nest.depth >= 1 && nest.depth <= 5 && Number.isInteger(nest.food) && nest.food >= 0 && nest.food <= nest.depth * 20) && new Set(state.nests.map((nest) => `${nest.x},${nest.y}`)).size === state.nests.length) && (state.food === void 0 || Array.isArray(state.food) && state.food.length <= 20 && state.food.every((food) => food && typeof food.id === "string" && food.id.length <= 32 && inHabitat(state, food.x, food.y) && Number.isInteger(food.portions) && food.portions >= 1 && food.portions <= 10 && Number.isFinite(food.droppedAtMs) && food.droppedAtMs <= state.updatedAtMs && (food.kind === void 0 || food.kind === "sunflower_seed" || food.kind === "flower_seed"))) && (state.webs === void 0 || Array.isArray(state.webs) && state.webs.length <= capacity(state).organisms * TERRARIUM_WEBS_PER_SPIDER && state.webs.every((web) => web && typeof web.id === "string" && web.id.length <= 32 && typeof web.spiderId === "string" && inHabitat(state, web.x, web.y) && Number.isFinite(web.createdAtMs) && web.createdAtMs <= state.updatedAtMs && state.bugs.some((bug) => bug.id === web.spiderId && bug.species === "spider")) && new Set(state.webs.map((web) => `${web.x},${web.y}`)).size === state.webs.length && state.bugs.every((bug) => state.webs.filter((web) => web.spiderId === bug.id).length <= TERRARIUM_WEBS_PER_SPIDER)) && Array.isArray(state.items) && state.items.length === 2 && ["coin", "pebble"].every((id) => state.items.filter((item) => item.id === id && inHabitat(state, item.x, item.y)).length === 1) && !(state.items[0].x === state.items[1].x && state.items[0].y === state.items[1].y) && new Set([...state.plants, ...state.seeds, ...state.bugs, ...state.eggs ?? [], ...state.nests ?? [], ...state.food ?? [], ...state.webs ?? [], ...(state.collection ?? []).map((e) => e.specimen)].map((v) => v.id)).size === (state.collection?.length ?? 0) + state.plants.length + state.seeds.length + state.bugs.length + (state.eggs?.length ?? 0) + (state.nests?.length ?? 0) + (state.food?.length ?? 0) + (state.webs?.length ?? 0) && new Set([...state.plants, ...state.seeds].map((v) => `${v.x},${v.y}`)).size === state.plants.length + state.seeds.length && validColony(state);
}

// ../terrarium-server/bridge/worker.ts
for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
  try {
    const raw = JSON.parse(line);
    const state = raw?.kind === "water" ? validAquarium(raw) ? raw : void 0 : migrateState(raw);
    process.stdout.write(JSON.stringify(state ? { state } : { invalid: true }) + "\n");
  } catch {
    process.stdout.write('{"invalid":true}\n');
  }
}
