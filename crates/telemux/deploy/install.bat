@echo off
rem Telmux-rs Windows 部署脚本（阶段 8）。
rem 用法（管理员 PowerShell/CMD）：
rem   deploy\install.bat [config.toml 绝对路径]
rem
rem 步骤：构建 release -> 复制二进制与配置到 C:\Program Files\Telemux -> 可选安装 Windows 服务。
setlocal enabledelayedexpansion

set ROOT=%~dp0..
set DEST=C:\Program Files\Telemux
set CONFIG=%~1
if "%CONFIG%"=="" set CONFIG=%ROOT%\config\example.toml

echo ==^> 1/4 构建 release 二进制
if not exist "%ROOT%\target\release\telemux.exe" (
    pushd "%ROOT%"
    cargo build --release
    popd
)

echo ==^> 2/4 创建安装目录
mkdir "%DEST%" 2>nul
copy /y "%ROOT%\target\release\telemux.exe" "%DEST%\telemux.exe" >nul

echo ==^> 3/4 复制配置
copy /y "%CONFIG%" "%DEST%\telemux.toml" >nul
echo     配置：%DEST%\telemux.toml（请按需编辑 log_dir、设备清单等）

echo ==^> 4/4 可选：安装为 Windows 服务
choice /c YN /m "安装为 Windows 服务（需管理员）"
if errorlevel 2 goto :done
"%DEST%\telemux.exe" --install-service --config "%DEST%\telemux.toml"
echo 启动服务：sc start telemux
goto :eof

:done
echo 完成。前台运行：%DEST%\telemux.exe --config "%DEST%\telemux.toml"
endlocal
