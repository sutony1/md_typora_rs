"""Integration tests: tail reads must equal a clean rebuild, including boundaries."""
import base64
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from codex_export import zstandard_module

HERE = pathlib.Path(__file__).resolve().parent
PNG = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aL1sAAAAASUVORK5CYII=')


def message(text, ident, stamp='2026-10-26T15:59:00Z', turn='t1', event=False, image=False):
    content = [{'type':'input_text', 'text':text}]
    if image:
        content.append({'type':'input_image', 'image_url':'data:image/png;base64,'+base64.b64encode(PNG).decode()})
    item = {'type':'message', 'role':'user', 'id':ident, 'content':content,
            'internal_chat_message_metadata_passthrough':{'turn_id':turn}}
    kind = 'response_item'
    if event:
        kind = 'event_msg'
        item['type'] = 'UserMessage'
        item = {'type':'item_completed', 'turn_id':turn, 'item':item}
    return {'timestamp':stamp, 'type':kind, 'payload':item}


def encoded(records):
    return b''.join(json.dumps(r, ensure_ascii=False).encode('utf-8')+b'\n' for r in records)


class IncrementalTest(unittest.TestCase):
    def setUp(self):
        self.work = pathlib.Path(tempfile.mkdtemp(prefix='incremental_check_', dir=HERE)).resolve()
        self.addCleanup(self.cleanup)
        self.path = self.work/'fixture.jsonl'
        self.target = self.work/'output'
        self.original = encoded([{'type':'session_meta', 'payload':{'id':'fixture-session'}},
                                 message('旧消息', 'old', turn='old')])
        self.path.write_bytes(self.original)

    def cleanup(self):
        assert self.work.parent == HERE and self.work.name.startswith('incremental_check_')
        shutil.rmtree(self.work, ignore_errors=True)

    def run_export(self, path=None, target=None, force=False):
        target = target or self.target
        command = [sys.executable, str(HERE/'codex_export.py'), str(path or self.path), '--output', str(target)]
        if force:
            command.append('--force')
        run = subprocess.run(command, capture_output=True, encoding='utf-8', timeout=30)
        self.assertEqual(run.returncode, 0, run.stdout+run.stderr)
        report = json.loads((target/'运行报告.json').read_text(encoding='utf-8'))
        return report['sessions'][0]

    def append(self, data):
        with self.path.open('ab') as stream:
            stream.write(data)

    def assert_matches_full(self):
        clean = self.work/'clean'
        self.run_export(target=clean, force=True)
        current = self.target/'fixture'
        reference = clean/'fixture'
        self.assertEqual({p.name for p in current.glob('*_对话.md')},
                         {p.name for p in reference.glob('*_对话.md')})
        for path in reference.glob('*_对话.md'):
            self.assertEqual(path.read_bytes(), (current/path.name).read_bytes())
        for path in (reference/'images').glob('*'):
            self.assertEqual(path.read_bytes(), (current/'images'/path.name).read_bytes())

    def test_tail_dates_images_and_cross_boundary_dedup(self):
        # Export a response first, then receive its UI completion next run.
        self.append(encoded([message('边界图片', 'response', turn='image', image=True)]))
        initial = self.run_export()
        day = self.target/'fixture'/'2026-10-26_对话.md'
        before = day.stat().st_mtime_ns
        appended = encoded([message('边界图片', 'ui', turn='image', event=True),
                            message('第二天的消息', 'new', stamp='2026-10-26T16:01:00Z', turn='new', image=True)])
        self.append(appended)
        report = self.run_export()
        self.assertEqual(report['mode'], 'incremental')
        self.assertEqual(report['bytes_read_this_run'], len(appended))
        self.assertEqual(report['read_from_offset'], initial['checkpoint_offset'])
        self.assertEqual(report['messages'], 3)
        self.assertEqual(report['image_references'], 2)
        self.assertNotEqual(day.stat().st_mtime_ns, before)  # UI completion replaces response source line.
        self.assert_matches_full()
        cached = self.run_export()
        self.assertEqual(cached['mode'], 'skipped')
        self.assertEqual(cached['bytes_read_this_run'], 0)

    def test_untouched_old_day_is_not_rewritten(self):
        self.run_export()
        old_day = self.target/'fixture'/'2026-10-26_对话.md'
        before = old_day.stat().st_mtime_ns
        self.append(encoded([message('新一天', 'next', stamp='2026-10-27T01:00:00Z', turn='next')]))
        self.assertEqual(self.run_export()['mode'], 'incremental')
        self.assertEqual(old_day.stat().st_mtime_ns, before)
        self.assert_matches_full()

    def test_partial_utf8_line_is_retried_without_loss(self):
        self.run_export()
        raw = encoded([message('未写完的中文图片', 'partial', image=True)])
        cut = raw.index('中'.encode('utf-8'))+1
        self.append(raw[:cut])
        report = self.run_export()
        self.assertEqual(report['messages'], 1)
        self.assertEqual(report['parse_errors'], [])
        self.assertEqual(report['checkpoint_offset'], len(self.original))
        self.assertEqual(report['deferred_bytes'], cut)
        self.append(raw[cut:])
        report = self.run_export()
        self.assertEqual(report['messages'], 2)
        self.assertEqual(report['bytes_read_this_run'], len(raw))
        self.assertEqual(report['deferred_bytes'], 0)
        self.assert_matches_full()

    def test_truncation_rebuilds_and_archives_stale_days(self):
        self.append(encoded([message('未来消息', 'future', stamp='2026-10-28T01:00:00Z')]))
        self.run_export()
        self.path.write_bytes(self.original)
        report = self.run_export()
        self.assertEqual(report['mode'], 'full')
        self.assertEqual(report['messages'], 1)
        self.assertFalse((self.target/'fixture'/'2026-10-28_对话.md').exists())
        self.assertTrue(list((self.target/'fixture').glob('更新前备份_*/2026-10-28_对话.md')))
        self.assert_matches_full()

    def test_deleted_image_and_broken_checkpoint_recover(self):
        self.append(encoded([message('图片', 'image', image=True)]))
        self.run_export()
        image = next((self.target/'fixture'/'images').glob('*.png'))
        image.unlink()
        self.assertEqual(self.run_export()['mode'], 'full')
        self.assertTrue(image.exists())
        (self.target/'fixture'/'.增量缓存.json').write_text('{broken', encoding='utf-8')
        self.assertEqual(self.run_export()['mode'], 'full')
        day = next((self.target/'fixture').glob('*_对话.md'))
        day.write_text('手动编辑的内容', encoding='utf-8')
        self.assertEqual(self.run_export()['mode'], 'full')
        self.assert_matches_full()

    def test_prefix_rewrite_and_append_rebuilds(self):
        self.run_export()
        edited = self.original.replace('旧消息'.encode(), '改消息'.encode())
        self.path.write_bytes(edited+encoded([message('新消息', 'new')]))
        self.assertEqual(self.run_export()['mode'], 'full')
        self.assert_matches_full()

    def test_force_and_compression_transition(self):
        self.run_export()
        self.append(encoded([message('新消息', 'new')]))
        self.assertEqual(self.run_export(force=True)['mode'], 'full')
        packed = self.work/'fixture.jsonl.zst'
        packed.write_bytes(zstandard_module().ZstdCompressor().compress(self.path.read_bytes()))
        self.assertEqual(self.run_export(path=packed)['mode'], 'full')
        self.assertEqual(self.run_export(path=packed)['mode'], 'skipped')
        self.append(encoded([message('压缩后又继续', 'again')]))
        self.assertEqual(self.run_export()['mode'], 'full')  # Storage format changed.
        self.append(encoded([message('恢复增量', 'resume')]))
        self.assertEqual(self.run_export()['mode'], 'incremental')
        self.assert_matches_full()


if __name__ == '__main__':
    unittest.main()
