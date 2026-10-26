"""Native file picker used only when the user launches the batch entry point."""
import pathlib
import subprocess
import sys
import tkinter as tk
from tkinter import filedialog

root = tk.Tk()
root.withdraw()
source = filedialog.askopenfilename(title='选择要导出的 Codex 会话记录',
                                   filetypes=[('Codex 会话', '*.jsonl *.jsonl.zst'),
                                              ('压缩会话', '*.jsonl.zst'),('JSONL', '*.jsonl')])
root.destroy()
if source:
    here = pathlib.Path(__file__).resolve().parent
    raise SystemExit(subprocess.call([sys.executable, str(here/'codex_export.py'), source,
                                     '--output', str(here/'导出结果')]))
