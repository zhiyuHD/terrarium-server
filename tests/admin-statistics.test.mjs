import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { runInNewContext } from 'node:vm';

const source = readFileSync(new URL('../web/admin/client.js', import.meta.url), 'utf8');
const functions = source.slice(source.indexOf('function statisticsDistribution('), source.indexOf('function statisticsChart('));
const { statisticsData, statisticsSlices } = runInNewContext(`${functions}\n({statisticsData, statisticsSlices})`);
const plain = value => JSON.parse(JSON.stringify(value));
const now = Date.parse('2026-01-02T00:00:00Z');
const boxes = [
  { reportedAt: new Date(now).toISOString(), clientVersion: '1.0.0', kind: 'land', clientSystem: { name: 'Linux', version: 'Example OS' }, coopanionVersion: '1.0.0', wsConnections: 2, reportTransport: 'websocket' },
  { reportedAt: new Date(now - 86400000).toISOString(), clientVersion: '1.0.0', kind: 'water', wsConnections: 0 },
  { reportedAt: new Date(now - 86400001).toISOString(), kind: 'land', reportTransport: 'http', wsConnections: 0 },
  { reportedAt: null, clientVersion: null, kind: null, wsConnections: 0 }
];

test('statistics count reported boxes, preserve missing fields, and do not infer Cortico from missing Coopanion', () => {
  const result = statisticsData(boxes, 'reported', now);
  assert.equal(result.total, 3);
  assert.equal(result.unreported, 1);
  assert.equal(result.reported, 3);
  const chart = title => plain(result.charts.find(chart => chart.title === title).rows);
  assert.deepEqual(chart('World 版本'), [{ label: '1.0.0', count: 2 }, { label: '≤0.7.3（推定）', count: 1 }]);
  assert.deepEqual(chart('部署识别'), [{ label: '未识别', count: 2 }, { label: 'Coopanion', count: 1 }]);
  assert.deepEqual(chart('操作系统'), [{ label: '未上报', count: 2 }, { label: 'Linux', count: 1 }]);
  assert.ok(result.charts.every(chart => chart.rows.reduce((sum, row) => sum + row.count, 0) === result.total));
});

test('activity scopes use a bounded 24-hour window and count boxes rather than WS connections', () => {
  assert.equal(statisticsData(boxes, 'recent', now).total, 2);
  assert.equal(statisticsData(boxes, 'online', now).total, 1);
  assert.equal(statisticsData([{ reportedAt: new Date(now + 1).toISOString() }], 'recent', now).total, 0);
  const empty = statisticsData([], 'reported', now);
  assert.equal(empty.total, 0);
  assert.ok(empty.charts.every(chart => chart.rows.length === 0));
});

test('pie chart grouping preserves the total and leaves complete distribution details intact', () => {
  const rows = Array.from({ length: 15 }, (_, index) => ({ label: String(index), count: 15 - index }));
  const slices = statisticsSlices(rows);
  assert.equal(slices.length, 9);
  assert.equal(slices.at(-1).label, '其他');
  assert.equal(slices.reduce((sum, row) => sum + row.count, 0), rows.reduce((sum, row) => sum + row.count, 0));
  assert.equal(rows.length, 15);
  assert.equal(statisticsSlices(rows.slice(0, 8)).length, 8);
});
