#!/bin/bash
# 桌面AI 便携版 (Linux 入口)
cd "$(dirname "$0")"
chmod +x desktop-ai 2>/dev/null || true
exec ./desktop-ai "$@"
