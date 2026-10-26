@echo off
chcp 65001 >nul
cd /d "%~dp0"
echo 安装 Zstandard 解压库到工具自己的 _deps 目录，需要联网。
python -m pip install --target "%~dp0_deps" -r "%~dp0requirements.txt"
pause
