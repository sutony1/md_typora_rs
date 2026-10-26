@echo off
chcp 65001 >nul
setlocal
cd /d "%~dp0"
set "PYTHONIOENCODING=utf-8"
python --version >nul 2>&1
if errorlevel 1 (
  echo 未找到 Python。请安装 Python 3.10 或更新版本，并勾选添加到 PATH。
  pause
  exit /b 1
)
if not "%~1"=="" goto dragged
echo.
echo Codex 对话导出工具
echo 1. 选择一个 JSONL 或 JSONL.ZST 文件
echo 2. 导出本机所有用户会话（自动过滤后台辅助记录）
echo 其他按键退出
choice /c 12Q /n /m "请选择 [1/2/Q]："
if errorlevel 3 exit /b 0
if errorlevel 2 goto all
python "%~dp0选择文件.py"
goto done
:all
python "%~dp0codex_export.py" --all --output "%~dp0导出结果"
goto done
:dragged
python "%~dp0codex_export.py" "%~1" --output "%~dp0导出结果"
:done
echo.
pause
