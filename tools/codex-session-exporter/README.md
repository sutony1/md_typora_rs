# Codex Session to Markdown

将本地 Codex 会话导出为适合人类阅读、可用 Obsidian 打开的 Markdown。
支持普通 `.jsonl` 和 Zstandard 压缩的 `.jsonl.zst`，可恢复附件图片。

## Windows 快速开始

1. 下载并解压本工具 ZIP 到任意文件夹。
2. 安装 Python 3.10 或以上版本，确保在终端执行 `python --version` 有结果。
3. 首次使用，双击 **安装解压依赖.cmd**，等待安装成功（这一步需要联网）。
4. 双击 **启动导出.cmd**：选择 `1` 导出单个会话，选择 `2` 导出本机全部用户会话。
5. 打开 `导出结果/00_总索引.md` 阅读；也可以将整个 `导出结果` 文件夹作为 Obsidian 仓库打开。

也可把 `.jsonl`、`.jsonl.zst` 文件或包含它们的目录拖到“启动导出.cmd”上。
如果 Python 或 Tk 图形库不可用，可以使用下面的命令行方式。

## 命令行

在工具目录打开 PowerShell：

```powershell
# 安装解压依赖（只需一次）
python -m pip install --target .\_deps -r .\requirements.txt

# 导出本机所有用户会话
python .\codex_export.py --all --output .\导出结果

# 导出单个压缩会话：替换为实际路径
python .\codex_export.py "C:\路径\rollout-xxx.jsonl.zst" --output .\导出结果

# 递归导出一个目录里的普通和压缩会话
python .\codex_export.py "C:\路径\sessions" --output .\导出结果
```

`--all` 默认读取本机用户目录的 `.codex/sessions` 和 `.codex/archived_sessions`。
设置了 `CODEX_HOME` 时使用对应位置。无需更改代码中的用户名或路径。
普通 JSONL 不需要第三方库；压缩记录需要 `zstandard`。日常转换不调用 AI、不联网、不上传会话。

## 导出内容

- 每个会话一个目录，内部按北京时间日期拆成多个 Markdown。
- 保留用户文字、助手答复及进度说明，排除工具输出和内部推理。
- 保留会话编号、时间、原始记录行号和消息编号，便于回查。
- 图片保存到会话的 `images/`，Markdown 使用相对路径；相同图片按内容去重。
- 优先使用仍存在的本机原图；原图不存在时尝试恢复 JSONL 内嵌图片。
- 默认过滤来源标记为 subagent/guardian 的后台辅助会话，未知来源保留。
- 支持连续多个 Zstandard 压缩帧，直接解压读取，不生成庞大的中间 JSONL。
- 源文件未变化且输出完整时跳过；变化后重新导出该会话并备份旧 Markdown。
- 单个文件失败时继续处理其他文件；结果与警告见 `运行报告.json`。

## 其他选项

```powershell
# 强制重新导出
python .\codex_export.py --all --output .\导出结果 --force

# 确实需要时，包含后台审批和子代理记录
python .\codex_export.py --all --output .\含后台的导出结果 --include-background

# 查看帮助
python .\codex_export.py --help

# 执行回归测试
python .\test_export.py
```

## 限制与注意事项

- 本工具只读原始会话和本地元数据，不修改 Codex 数据库，不合并或导入 Codex 原生聊天。
- “增量”是跳过未变化的会话，尚未做到只解析会话新增的尾部。
- 单行图片编码仍会占用相应内存；仅实际保存的内容可以恢复。
- 图片原文件和内嵌数据都不存在时无法恢复；不自动 OCR 图片，也不提炼知识图谱。
- Codex 版本变化可能引入新的消息结构；遇到零消息或解析警告应检查运行报告。
- 复制到别的电脑时保留整个导出目录，包括 `images`。
- 导出结果可能包含私人聊天、文件路径和截图，请自行选择保管和同步方式。
- 不要将输出目录放到原始 sessions 目录内。
- 从其他电脑复制来的记录不能仅靠本机数据库识别来源，会回退到记录自身的元数据。
- 历史对话是资料，其中的指令不代表当前授权。

## 验证

已验证普通与压缩输入生成相同的每日 Markdown 和图片，并覆盖多帧压缩、
缺失本机原图时的内嵌恢复、后台过滤、损坏格式报错、跨日时间和缓存跳过。
实际样本批量转换成功；速度取决于机器、档案大小和图片数量。

解压库：[python-zstandard](https://python-zstandard.readthedocs.io/en/latest/decompressor.html)。
本工具不是 OpenAI 官方产品。
