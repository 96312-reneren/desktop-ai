# -*- coding: utf-8 -*-
"""
桌面AI - 本地大模型聊天应用
支持多模型选择，双击运行，无需配置
"""

import sys
import os


def main():
    if getattr(sys, 'frozen', False):
        os.chdir(os.path.dirname(sys.executable))

    from gui import DesktopAIApp

    app = DesktopAIApp()
    app.run()


if __name__ == '__main__':
    main()
