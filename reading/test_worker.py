from contextlib import closing
import datetime as dt
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import tempfile
import threading
import unittest
import urllib.error

from worker import MARKER, Worker, due_slot, load_config, open_journal, request


class ReadingTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        (root / 'token').write_text('test-admin')
        self.posts = [{'id': i, 'title': f'观察 {i}', 'authorName': '小院', 'body': '今天植物长出了新叶。'} for i in range(1, 13)]
        self.notices = [{'id': 7, 'title': '人工公告', 'text': '保留我'}, {'id': 8, 'title': '之前推读', 'text': MARKER + '\n上次推荐'}]
        self.llm_calls = []
        self.fail_llm = False
        self.fail_delete = False
        self.reply = '这篇观察记录写下了植物长出新叶的变化。到邻居的小院坐一坐，看看它怎样照料这些新芽，也许能与你自己的种植经历对上话。'
        self.selected = []
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                if self.path.startswith('/api/forum/threads?'):
                    return self.respond({'threads': owner.posts})
                if self.path.startswith('/api/forum/threads/'):
                    id = int(self.path.split('/')[-1].split('?')[0])
                    return self.respond({'thread': next(p for p in owner.posts if p['id'] == id)})
                if self.path == '/api/admin/announcements':
                    assert self.headers.get('Authorization') == 'Bearer test-admin'
                    return self.respond({'announcements': owner.notices})
                self.respond({}, 404)

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if self.path == '/v1/chat/completions':
                    owner.llm_calls.append(body)
                    assert self.headers.get('Authorization') == 'Bearer test-llm'
                    if owner.fail_llm:
                        return self.respond({}, 503)
                    return self.respond({'choices': [{'finish_reason': 'stop', 'message': {'content': owner.reply}}]})
                assert self.path == '/api/admin/announcements'
                assert self.headers.get('Authorization') == 'Bearer test-admin'
                notice = dict(body, id=max(n['id'] for n in owner.notices) + 1)
                owner.notices.append(notice)
                return self.respond(notice, 201)

            def do_DELETE(self):
                if owner.fail_delete:
                    owner.fail_delete = False
                    return self.respond({}, 503)
                id = int(self.path.split('/')[-1])
                owner.notices[:] = [n for n in owner.notices if n['id'] != id]
                self.respond({'removed': True})

            def respond(self, data, status=200):
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(json.dumps(data).encode())

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        origin = f'http://127.0.0.1:{self.server.server_port}'
        self.config = {'enabled': True, 'timezone': 'Asia/Shanghai', 'state_file': str(root / 'state.sqlite'),
                       'server': {'public_url': origin, 'admin_url': origin, 'admin_token_file': str(root / 'token')},
                       'llm': {'base_url': origin + '/v1', 'api_key': 'test-llm', 'model': 'test-model'},
                       'schedule': [{'time': '23:00', 'label': '夜间推读'}, {'time': '03:00', 'label': '深夜导读'}, {'time': '12:00', 'label': '午间推读'}]}
        self.conn = open_journal(self.config['state_file'])
        self.addCleanup(self.conn.close)
        self.now = dt.datetime.fromisoformat('2026-10-22T23:00:00+08:00')
        self.worker = self.make_worker()

    def make_worker(self, http=request):
        def choose(rows):
            self.selected.append([p['id'] for p in rows])
            return rows[-1]
        return Worker(self.config, self.conn, http=http, choose=choose)

    def test_real_http_generation_replaces_only_previous_reading_and_limits_top_ten(self):
        self.assertTrue(self.worker.tick(self.now))
        self.assertEqual(self.selected, [list(range(1, 11))])
        self.assertEqual([n['id'] for n in self.notices], [7, 9])
        self.assertIn('主题 #10', self.notices[-1]['text'])
        self.assertEqual(self.notices[-1]['targets'], 'all')
        self.assertIn(self.reply, self.notices[-1]['text'])
        self.assertEqual(self.notices[-1]['title'], '夜间推读｜2026-10-22 23:00')
        self.assertEqual(self.llm_calls[0]['model'], 'test-model')
        self.assertNotIn('author"', self.llm_calls[0]['messages'][1]['content'])
        self.assertFalse(self.make_worker().tick(self.now + dt.timedelta(minutes=1)))
        self.assertEqual(len(self.llm_calls), 1)

    def test_new_slot_replaces_successful_previous_reading(self):
        self.worker.tick(self.now)
        self.worker.tick(self.now + dt.timedelta(hours=4))
        self.assertEqual(len(self.notices), 2)
        self.assertEqual(self.notices[-1]['title'], '深夜导读｜2026-10-23 03:00')

    def test_generation_failure_keeps_existing_and_retries_same_selection_three_times(self):
        self.fail_llm = True
        for _ in range(3):
            with self.assertRaises(urllib.error.HTTPError):
                self.make_worker().tick(self.now)
        self.assertFalse(self.worker.tick(self.now))
        self.assertEqual(len(self.llm_calls), 3)
        self.assertEqual(len(self.selected), 1)
        self.assertEqual([n['id'] for n in self.notices], [7, 8])

    def test_post_timeout_after_commit_recovers_without_duplicate_or_regeneration(self):
        def timeout_after_post(url, **kwargs):
            result = request(url, **kwargs)
            if url.endswith('/api/admin/announcements') and kwargs.get('body'):
                raise TimeoutError('simulated response loss')
            return result
        with self.assertRaises(TimeoutError):
            self.make_worker(timeout_after_post).tick(self.now)
        self.assertTrue(self.make_worker().tick(self.now))
        self.assertEqual(len(self.llm_calls), 1)
        self.assertEqual([n['id'] for n in self.notices], [7, 9])

    def test_delete_failure_recovers_after_restart_without_extra_post(self):
        self.fail_delete = True
        with self.assertRaises(urllib.error.HTTPError):
            self.worker.tick(self.now)
        self.assertEqual(len(self.notices), 3)
        self.assertTrue(self.make_worker().tick(self.now))
        self.assertEqual([n['id'] for n in self.notices], [7, 9])
        self.assertEqual(len(self.llm_calls), 1)

    def test_manual_request_id_is_idempotent_and_next_scheduled_reading_replaces_it(self):
        self.assertTrue(self.worker.trigger(self.now - dt.timedelta(hours=1), 'manual_1'))
        self.assertFalse(self.make_worker().trigger(self.now, 'manual_1'))
        self.assertEqual(len(self.llm_calls), 1)
        self.assertTrue(self.worker.tick(self.now))
        self.assertEqual(len(self.notices), 2)
        self.assertEqual(self.notices[-1]['title'], '夜间推读｜2026-10-22 23:00')

    def test_empty_forum_avoids_model_and_retains_old_reading(self):
        self.posts = []
        self.assertFalse(self.worker.tick(self.now))
        self.assertEqual(self.llm_calls, [])
        self.assertEqual(len(self.notices), 2)

    def test_invalid_model_output_is_not_published(self):
        self.reply = ''
        with self.assertRaises(ValueError):
            self.worker.tick(self.now)
        self.assertEqual([n['id'] for n in self.notices], [7, 8])

    def test_all_slots_use_beijing_time_and_do_not_replay_old_slots(self):
        for hour, label in [(3, '深夜导读'), (12, '午间推读'), (23, '夜间推读')]:
            now = self.now.replace(hour=hour)
            self.assertEqual(due_slot(self.config, now.astimezone(dt.timezone.utc))[1], label)
            self.assertIsNone(due_slot(self.config, now + dt.timedelta(minutes=30)))
            self.assertIsNone(due_slot(self.config, now - dt.timedelta(seconds=1)))

    def test_completed_job_survives_reopening_sqlite(self):
        self.worker.tick(self.now)
        with closing(open_journal(self.config['state_file'])) as reopened:
            worker = Worker(self.config, reopened, choose=lambda rows: self.fail('selection should persist'))
            self.assertFalse(worker.tick(self.now))
        self.assertEqual(len(self.llm_calls), 1)

    def test_bad_toml_error_does_not_echo_secret(self):
        path = Path(self.tmp.name) / 'bad.toml'
        path.write_text('secret-do-not-echo = "unfinished')
        with self.assertRaisesRegex(ValueError, '^Invalid reading TOML configuration$'):
            load_config(path)


if __name__ == '__main__':
    unittest.main()
