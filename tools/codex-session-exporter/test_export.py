"""Regression: duplicate messages, missing local image fallback, dates and caching."""
import base64
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from codex_export import zstandard_module

HERE = pathlib.Path(__file__).resolve().parent


class ExportTest(unittest.TestCase):
    def test_recovery_dates_and_cache(self):
        work = pathlib.Path(tempfile.mkdtemp(prefix='export_check_', dir=HERE))
        source = work/'source'
        source.mkdir()
        target = work/'output'
        png = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aL1sAAAAASUVORK5CYII=')
        image = {'type':'input_image', 'image_url':'data:image/png;base64,'+base64.b64encode(png).decode()}
        records = [
            {'type':'session_meta', 'payload':{'id':'fixture-session'}},
            {'type':'response_item', 'payload':{'type':'message','role':'user','id':'response-user',
                'internal_chat_message_metadata_passthrough':{'turn_id':'t1'},
                'content':[{'type':'input_text','text':'测试图片<image name="x"></image>'},image]}},
            {'type':'event_msg', 'payload':{'type':'item_completed','turn_id':'t1','item':{
                'type':'UserMessage','id':'ui-user','content':[{'type':'text','text':'测试图片'},
                {'type':'local_image','path':str(work/'missing.png')}]}}},
            {'type':'response_item', 'payload':{'type':'message','role':'assistant','channel':'analysis',
                'content':[{'type':'output_text','text':'INTERNAL_REASONING_MUST_NOT_APPEAR'}]}},
            {'type':'response_item', 'payload':{'type':'message','role':'assistant','id':'final',
                'phase':'final_answer','content':[{'type':'output_text','text':'收到图片'}]}},
            {'type':'response_item', 'payload':{'type':'message','role':'user','id':'only-image','content':[image]}},
        ]
        for index, record in enumerate(records):
            record['timestamp'] = '2026-10-01T15:59:59Z' if index < 4 else '2026-10-01T16:01:00Z'
        path = source/'fixture.jsonl'
        path.write_text('\n'.join(json.dumps(r,ensure_ascii=False) for r in records)+'\n',encoding='utf-8')
        command = [sys.executable,str(HERE/'codex_export.py'),str(source),'--output',str(target)]
        first = subprocess.run(command,capture_output=True,encoding='utf-8')
        self.assertEqual(first.returncode,0,first.stdout+first.stderr)
        report = json.loads((target/'运行报告.json').read_text(encoding='utf-8'))
        session = report['sessions'][0]
        self.assertEqual(session['messages'],3)
        self.assertEqual(len(session['days']),2)
        self.assertEqual(session['image_references'],2)
        markdown = '\n'.join(p.read_text(encoding='utf-8-sig') for p in (target/'fixture').glob('*_对话.md'))
        self.assertNotIn('INTERNAL_REASONING',markdown)
        self.assertNotIn('data:image/',markdown)
        self.assertEqual(len(list((target/'fixture'/'images').glob('*.png'))),1)
        self.assertEqual(next((target/'fixture'/'images').glob('*.png')).read_bytes(),png)
        second = subprocess.run(command,capture_output=True,encoding='utf-8')
        self.assertEqual(second.returncode,0,second.stdout+second.stderr)
        self.assertEqual(json.loads((target/'运行报告.json').read_text(encoding='utf-8'))['skipped'],1)
        # Real Zstandard compression, including concatenated frames; the daily
        # Markdown and recovered image bytes must match the plain input exactly.
        zstd = zstandard_module()
        compressed_source = work/'compressed'
        compressed_source.mkdir()
        raw = path.read_bytes()
        split = len(raw)//2
        packed = zstd.ZstdCompressor().compress(raw[:split])+zstd.ZstdCompressor().compress(raw[split:])
        packed_path = compressed_source/'fixture.jsonl.zst'
        packed_path.write_bytes(packed)
        compressed_target = work/'compressed_output'
        compressed_command = [sys.executable,str(HERE/'codex_export.py'),str(packed_path),'--output',str(compressed_target)]
        run = subprocess.run(compressed_command,capture_output=True,encoding='utf-8')
        self.assertEqual(run.returncode,0,run.stdout+run.stderr)
        self.assertEqual(packed_path.read_bytes(),packed)
        for original in (target/'fixture').glob('*_对话.md'):
            self.assertEqual(original.read_bytes(),(compressed_target/'fixture'/original.name).read_bytes())
        self.assertEqual(next((compressed_target/'fixture'/'images').glob('*.png')).read_bytes(),png)
        compressed_report = json.loads((compressed_target/'运行报告.json').read_text(encoding='utf-8'))
        self.assertEqual(compressed_report['sessions'][0]['snapshot_sha256'],session['snapshot_sha256'])
        # Default classification uses archive metadata even away from this PC's DB.
        background = work/'background'
        background.mkdir()
        records[0]['payload']['source'] = {'subagent':{'other':'guardian'}}
        data = '\n'.join(json.dumps(r,ensure_ascii=False) for r in records).encode('utf-8')
        (background/'guardian.jsonl.zst').write_bytes(zstd.ZstdCompressor().compress(data))
        bg_target = work/'background_output'
        bg_command = [sys.executable,str(HERE/'codex_export.py'),str(background),'--output',str(bg_target)]
        run = subprocess.run(bg_command,capture_output=True,encoding='utf-8')
        self.assertEqual(run.returncode,0,run.stdout+run.stderr)
        bg_report = json.loads((bg_target/'运行报告.json').read_text(encoding='utf-8'))
        self.assertEqual(bg_report['successful'],0)
        self.assertEqual(len(bg_report['filtered']),1)
        # A damaged archive must fail visibly rather than produce an empty success.
        broken = work/'broken.jsonl.zst'
        broken.write_bytes(b'not a zstandard frame')
        run = subprocess.run([sys.executable,str(HERE/'codex_export.py'),str(broken),
                              '--output',str(work/'broken_output')],capture_output=True,encoding='utf-8')
        self.assertEqual(run.returncode,1)
        print('测试数据保留在：'+str(work))


if __name__ == '__main__':
    unittest.main()
