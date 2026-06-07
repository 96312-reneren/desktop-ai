# -*- coding: utf-8 -*-
"""
桌面AI - 本地大模型聊天应用 (v2 - 性能优化版)
核心优化: streaming 期间纯文本追加，完成时一次性 Markdown 渲染
"""

import re
import threading
import customtkinter as ctk
from tkinter import messagebox
from pathlib import Path

from model import (
    load_config, save_config, get_models_dir,
    ModelDownloader, LLM, ConversationManager, get_model_info,
    get_downloaded_models
)

APP_TITLE = "桌面AI"
WINDOW_WIDTH = 1000
WINDOW_HEIGHT = 680
SIDEBAR_WIDTH = 220
MIN_WINDOW_WIDTH = 750
MIN_WINDOW_HEIGHT = 500

# 预编译正则
_RE_CODE_SPLIT = re.compile(r'(`[^`]+`|\*\*[^*]+\*\*)')
_RE_LIST_MARKER = re.compile(r'^[\-\*] ')
_RE_ORDERED_LIST = re.compile(r'^\d+\. ')


# ─── Markdown 渲染器（优化版）────────────────────────────

class MarkdownRenderer:
    """Markdown → CTkTextbox 渲染，标签只配置一次"""

    @staticmethod
    def setup_tags(textbox, font_size):
        """在 textbox 创建时一次性配置所有标签样式"""
        tb = textbox._textbox
        tb.tag_config('bold', font=('Microsoft YaHei', font_size, 'bold'))
        tb.tag_config('h1', font=('Microsoft YaHei', font_size + 6, 'bold'))
        tb.tag_config('h2', font=('Microsoft YaHei', font_size + 4, 'bold'))
        tb.tag_config('h3', font=('Microsoft YaHei', font_size + 2, 'bold'))
        tb.tag_config('code_inline',
                      font=('Consolas', font_size - 1),
                      background='#3a3a3a', foreground='#e6db74')
        tb.tag_config('code_block',
                      font=('Consolas', font_size - 1),
                      background='#2a2a2a', foreground='#e0e0e0',
                      lmargin1=10, lmargin2=10, spacing1=2, spacing3=2)
        tb.tag_config('blockquote',
                      font=('Microsoft YaHei', font_size, 'italic'),
                      foreground='#a0a0a0', lmargin1=15, lmargin2=15)
        tb.tag_config('list_item', lmargin1=15, lmargin2=25)

    @staticmethod
    def render(textbox, markdown_text, font_size=14):
        """将 markdown 文本渲染到 textbox（标签已预配置）"""
        textbox.configure(state='normal')
        textbox.delete('1.0', 'end')

        lines = markdown_text.split('\n')
        in_code_block = False
        code_lines = []

        for line in lines:
            stripped = line.strip()

            if stripped.startswith('```'):
                if in_code_block:
                    if code_lines:
                        textbox.insert('end', '\n'.join(code_lines) + '\n', 'code_block')
                    code_lines = []
                    in_code_block = False
                else:
                    in_code_block = True
                continue

            if in_code_block:
                code_lines.append(line)
                continue

            if stripped == '':
                textbox.insert('end', '\n')
                continue

            if line.startswith('### '):
                textbox.insert('end', line[4:] + '\n', 'h3')
            elif line.startswith('## '):
                textbox.insert('end', line[3:] + '\n', 'h2')
            elif line.startswith('# '):
                textbox.insert('end', line[2:] + '\n', 'h1')
            elif stripped in ('---', '***', '___'):
                textbox.insert('end', chr(0x2500) * 40 + '\n')
            elif line.startswith('> '):
                textbox.insert('end', chr(0x258E) + ' ' + line[2:] + '\n', 'blockquote')
            elif _RE_LIST_MARKER.match(line):
                MarkdownRenderer._insert_inline(textbox, '  ' + chr(0x2022) + ' ' + line[2:] + '\n',
                                                'list_item')
            elif _RE_ORDERED_LIST.match(line):
                textbox.insert('end', line + '\n', 'list_item')
            else:
                MarkdownRenderer._insert_inline(textbox, line + '\n')

        if in_code_block and code_lines:
            textbox.insert('end', '\n'.join(code_lines) + '\n', 'code_block')

        textbox.configure(state='disabled')

    @staticmethod
    def _insert_inline(textbox, text, tag_base=None):
        parts = _RE_CODE_SPLIT.split(text)
        for part in parts:
            if not part:
                continue
            if part.startswith('`') and part.endswith('`'):
                tags = ('code_inline',) if not tag_base else (tag_base, 'code_inline')
                textbox.insert('end', part[1:-1], tags)
            elif part.startswith('**') and part.endswith('**'):
                tags = ('bold',) if not tag_base else (tag_base, 'bold')
                textbox.insert('end', part[2:-2], tags)
            else:
                if tag_base:
                    textbox.insert('end', part, tag_base)
                else:
                    textbox.insert('end', part)


# ─── 模型选择界面 ───────────────────────────────────────

class ModelSelectDialog(ctk.CTkToplevel):

    def __init__(self, master, config):
        super().__init__(master)
        self.config = config
        self.selected_model_id = None
        self._downloader = ModelDownloader()
        self._active_download_id = None

        self.title("选择模型")
        self.geometry("620x480")
        self.resizable(False, False)
        self.grab_set()
        self.configure(fg_color=self._bg())

        ctk.CTkLabel(self, text="欢迎使用 桌面AI",
                     font=ctk.CTkFont(size=20, weight='bold')).pack(pady=(20, 0))
        ctk.CTkLabel(self, text="请选择一个模型开始使用（可后续在设置中更换）",
                     font=ctk.CTkFont(size=12), text_color='gray').pack(pady=(2, 15))

        self.scroll = ctk.CTkScrollableFrame(self, fg_color='transparent', width=580, height=300)
        self.scroll.pack(fill='both', expand=True, padx=15, pady=5)

        downloaded = get_downloaded_models(config)
        downloaded_ids = {m['id'] for m in downloaded}

        for model in config.get('model_catalog', []):
            self._create_model_card(model, model['id'] in downloaded_ids)

        btn_frame = ctk.CTkFrame(self, fg_color='transparent')
        btn_frame.pack(pady=(5, 15))
        ctk.CTkButton(btn_frame, text="跳过（稍后选择）", width=140,
                      fg_color='gray', command=self._on_skip).pack(side='left', padx=5)
        ctk.CTkButton(btn_frame, text="自定义下载地址（高级）", width=180,
                      fg_color='transparent', text_color='gray',
                      command=self._on_custom_url).pack(side='left', padx=5)

        self.protocol("WM_DELETE_WINDOW", self._on_skip)

    def _bg(self):
        return self.master._apply_appearance_mode(
            ctk.ThemeManager.theme["CTkFrame"]["fg_color"])

    def _create_model_card(self, model, is_downloaded):
        card = ctk.CTkFrame(self.scroll, corner_radius=10)
        card.pack(fill='x', pady=3)

        info_frame = ctk.CTkFrame(card, fg_color='transparent')
        info_frame.pack(side='left', fill='x', expand=True, padx=12, pady=8)

        name_row = ctk.CTkFrame(info_frame, fg_color='transparent')
        name_row.pack(anchor='w')
        ctk.CTkLabel(name_row, text=model['name'],
                     font=ctk.CTkFont(size=14, weight='bold')).pack(side='left')

        for tag in model.get('tags', []):
            tf = ctk.CTkFrame(name_row, fg_color='#1f6aa5', corner_radius=4, height=20)
            tf.pack(side='left', padx=2)
            ctk.CTkLabel(tf, text=tag, font=ctk.CTkFont(size=9),
                         text_color='white').pack(padx=4, pady=1)

        ctk.CTkLabel(info_frame, text=model['desc'],
                     font=ctk.CTkFont(size=11), text_color='gray',
                     justify='left', wraplength=380).pack(anchor='w', pady=(3, 0))
        ctk.CTkLabel(info_frame, text=f"约 {model['size_gb']} GB",
                     font=ctk.CTkFont(size=11), text_color='#4caf50').pack(anchor='w')

        btn_frame = ctk.CTkFrame(card, fg_color='transparent')
        btn_frame.pack(side='right', padx=10, pady=8)
        card.model_id = model['id']
        card.model_info = model

        if is_downloaded:
            self._show_downloaded(card, model)
        else:
            self._show_download_btn(card, model)

    def _show_downloaded(self, card, model):
        for w in card.winfo_children():
            if isinstance(w, ctk.CTkFrame):
                for c in w.winfo_children():
                    if isinstance(c, ctk.CTkFrame):
                        for b in c.winfo_children():
                            b.destroy()

        bf = ctk.CTkFrame(card, fg_color='transparent')
        bf.pack(side='right', padx=10, pady=8)
        ctk.CTkLabel(bf, text="已下载 \u2713", font=ctk.CTkFont(size=12),
                     text_color='#4caf50').pack(pady=(0, 5))
        ctk.CTkButton(bf, text="使用此模型", width=100,
                      command=lambda: self._on_select(model['id'])).pack()

    def _show_download_btn(self, card, model):
        for w in card.winfo_children():
            if isinstance(w, ctk.CTkFrame):
                for c in w.winfo_children():
                    if isinstance(c, ctk.CTkFrame):
                        for b in c.winfo_children():
                            b.destroy()

        bf = ctk.CTkFrame(card, fg_color='transparent')
        bf.pack(side='right', padx=10, pady=8)
        ctk.CTkButton(bf, text="下载", width=80,
                      command=lambda: self._start_download(card, model)).pack(pady=(0, 5))
        ctk.CTkButton(bf, text="使用（需已下载）", width=130,
                      fg_color='gray', state='disabled',
                      command=lambda: self._on_select(model['id'])).pack()

    def _start_download(self, card, model):
        if self._active_download_id is not None:
            messagebox.showinfo("提示", "已有模型正在下载中")
            return

        self._active_download_id = model['id']
        for w in card.winfo_children():
            if isinstance(w, ctk.CTkFrame):
                for c in w.winfo_children():
                    if isinstance(c, ctk.CTkFrame):
                        for b in c.winfo_children():
                            b.destroy()

        bf = ctk.CTkFrame(card, fg_color='transparent')
        bf.pack(side='right', padx=10, pady=8)
        progress = ctk.CTkProgressBar(bf, width=120)
        progress.pack(pady=(0, 5))
        progress.set(0)
        status_label = ctk.CTkLabel(bf, text="准备下载...", font=ctk.CTkFont(size=10))
        status_label.pack()
        ctk.CTkButton(bf, text="取消", width=60, fg_color='#8b3a3a',
                      command=lambda: self._cancel_download(card, model)).pack(pady=(5, 0))

        def _run():
            def on_progress(pct):
                self.after(0, lambda: progress.set(pct / 100.0))
            def on_status(text):
                self.after(0, lambda: status_label.configure(text=text))

            success = self._downloader.download(model,
                                                progress_callback=on_progress,
                                                status_callback=on_status)
            self.after(0, lambda: self._on_download_done(card, model, success))

        threading.Thread(target=_run, daemon=True).start()

    def _cancel_download(self, card, model):
        self._downloader.cancel()
        self._active_download_id = None
        self._show_download_btn(card, model)

    def _on_download_done(self, card, model, success):
        self._active_download_id = None
        if success:
            self._show_downloaded(card, model)
        else:
            self._show_download_btn(card, model)

    def _on_select(self, model_id):
        model = get_model_info(self.config, model_id)
        if model:
            path = get_models_dir() / model['filename']
            if path.exists():
                self.selected_model_id = model_id
                self.destroy()
            else:
                messagebox.showinfo("提示", "请先下载此模型")

    def _on_skip(self):
        downloaded = get_downloaded_models(self.config)
        if downloaded:
            self.selected_model_id = downloaded[0]['id']
        self.destroy()

    def _on_custom_url(self):
        messagebox.showinfo("自定义地址",
                            "如需使用自定义下载地址，请在软件设置中修改。\n\n"
                            "操作：关闭本窗口 → 点击右上角 ⚙ 设置")


# ─── 设置对话框 ─────────────────────────────────────────

class SettingsDialog(ctk.CTkToplevel):

    def __init__(self, master, config):
        super().__init__(master)
        self.config = config
        self.result_config = dict(config)

        self.title("设置")
        self.geometry("440x500")
        self.resizable(False, False)
        self.grab_set()
        self.configure(fg_color=self._bg())

        ctk.CTkLabel(self, text="设置", font=ctk.CTkFont(size=18, weight='bold')).pack(pady=(15, 10))

        ctk.CTkLabel(self, text="当前模型", font=ctk.CTkFont(size=13)).pack(anchor='w', padx=30)
        sel = self.config.get('selected_model_id')
        cur_name = get_model_info(self.config, sel)['name'] if sel and get_model_info(self.config, sel) else '未选择'
        ctk.CTkLabel(self, text=cur_name, font=ctk.CTkFont(size=12),
                     text_color='#4caf50').pack(anchor='w', padx=40, pady=(2, 5))
        ctk.CTkButton(self, text="切换模型", width=120, height=28,
                      command=self._switch_model).pack(pady=(0, 12))

        ctk.CTkLabel(self, text="主题外观", font=ctk.CTkFont(size=13)).pack(anchor='w', padx=30)
        self.theme_var = ctk.StringVar(value=config.get('theme', 'dark'))
        ctk.CTkOptionMenu(self, values=['dark', 'light'],
                          variable=self.theme_var, width=200).pack(pady=(2, 10))

        ctk.CTkLabel(self, text="字号", font=ctk.CTkFont(size=13)).pack(anchor='w', padx=30)
        ff = ctk.CTkFrame(self, fg_color='transparent')
        ff.pack(pady=(2, 10))
        self.font_var = ctk.IntVar(value=config.get('font_size', 14))
        ctk.CTkButton(ff, text="-", width=30,
                      command=lambda: self._adj(-1)).pack(side='left', padx=3)
        self.font_lbl = ctk.CTkLabel(ff, text=str(self.font_var.get()), width=30)
        self.font_lbl.pack(side='left')
        ctk.CTkButton(ff, text="+", width=30,
                      command=lambda: self._adj(1)).pack(side='left', padx=3)

        ctk.CTkLabel(self, text="上下文长度", font=ctk.CTkFont(size=13)).pack(anchor='w', padx=30)
        self.ctx_var = ctk.StringVar(value=str(config.get('n_ctx', 4096)))
        ctk.CTkEntry(self, textvariable=self.ctx_var, width=200).pack(pady=(2, 10))

        ctk.CTkLabel(self, text="CPU 线程数 (auto = 自动)", font=ctk.CTkFont(size=13)).pack(anchor='w', padx=30)
        self.thr_var = ctk.StringVar(value=str(config.get('n_threads', 'auto')))
        ctk.CTkEntry(self, textvariable=self.thr_var, width=200).pack(pady=(2, 10))

        ctk.CTkLabel(self, text="已下载的模型", font=ctk.CTkFont(size=13)).pack(anchor='w', padx=30)
        dl = get_downloaded_models(config)
        if dl:
            for m in dl:
                mf = ctk.CTkFrame(self, fg_color='transparent')
                mf.pack(anchor='w', padx=40, pady=1)
                ctk.CTkLabel(mf, text=f"{m['name']} ({m['size_gb']} GB)",
                             font=ctk.CTkFont(size=11)).pack(side='left')
                ctk.CTkButton(mf, text="删除", width=50, height=22, fg_color='#8b3a3a',
                              command=lambda mi=m: self._delete_model(mi)).pack(side='left', padx=(8, 0))
        else:
            ctk.CTkLabel(self, text="暂无已下载的模型",
                         font=ctk.CTkFont(size=11), text_color='gray').pack(anchor='w', padx=40)

        bf = ctk.CTkFrame(self, fg_color='transparent')
        bf.pack(pady=(12, 15))
        ctk.CTkButton(bf, text="保存", width=100, command=self._save).pack(side='left', padx=5)
        ctk.CTkButton(bf, text="取消", width=100, fg_color='gray',
                      command=self.destroy).pack(side='left', padx=5)

    def _bg(self):
        return self.master._apply_appearance_mode(
            ctk.ThemeManager.theme["CTkFrame"]["fg_color"])

    def _adj(self, d):
        v = self.font_var.get() + d
        if 10 <= v <= 24:
            self.font_var.set(v)
            self.font_lbl.configure(text=str(v))

    def _switch_model(self):
        dlg = ModelSelectDialog(self.master, self.config)
        self.master.wait_window(dlg)
        if dlg.selected_model_id:
            self.config['selected_model_id'] = dlg.selected_model_id
            save_config(self.config)
            self.destroy()
            self.master.event_generate('<<ReloadModel>>')

    def _delete_model(self, model):
        if messagebox.askyesno("确认", f"删除 {model['name']} 模型文件（{model['size_gb']} GB）？"):
            path = get_models_dir() / model['filename']
            if path.exists():
                path.unlink()
            messagebox.showinfo("完成", "模型已删除")
            self.destroy()
            SettingsDialog(self.master, self.config)

    def _save(self):
        self.result_config['theme'] = self.theme_var.get()
        self.result_config['font_size'] = self.font_var.get()
        try:
            self.result_config['n_ctx'] = int(self.ctx_var.get())
        except ValueError:
            pass
        self.result_config['n_threads'] = 'auto' if self.thr_var.get().lower() == 'auto' else (
            int(self.thr_var.get()) if self.thr_var.get().isdigit() else 'auto')
        self.destroy()


# ─── 主界面（v2 优化版）─────────────────────────────────

class DesktopAIApp:

    def __init__(self):
        self.root = ctk.CTk()
        self.root.title(APP_TITLE)
        self.root.geometry(f"{WINDOW_WIDTH}x{WINDOW_HEIGHT}")
        self.root.minsize(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)

        self.config = load_config()
        self.font_size = self.config.get('font_size', 14)

        ctk.set_appearance_mode(self.config.get('theme', 'dark'))
        ctk.set_default_color_theme("blue")

        self.conv_manager = ConversationManager()
        self.downloader = ModelDownloader()
        self.llm = LLM(self.config)

        self.is_generating = False
        self._stop_event = threading.Event()
        self._current_ai_textbox = None
        self._stream_buffer = []
        self._stream_scheduled = False

        self._build_ui()
        self.root.protocol("WM_DELETE_WINDOW", self._on_close)
        self.root.bind('<<ReloadModel>>', lambda e: self._reload_model())
        self.root.after(100, self._on_startup)

    # ─── UI 构建 ────────────────────────────────────────

    def _build_ui(self):
        self.root.grid_columnconfigure(1, weight=1)
        for i in range(4):
            self.root.grid_rowconfigure(i, weight=0)
        self.root.grid_rowconfigure(1, weight=1)

        self._build_sidebar()
        self._build_top_bar()
        self._build_chat_area()
        self._build_input_area()
        self._build_bottom_bar()

    def _build_sidebar(self):
        sidebar = ctk.CTkFrame(self.root, width=SIDEBAR_WIDTH, corner_radius=0)
        sidebar.grid(row=0, column=0, rowspan=4, sticky='ns')
        sidebar.grid_propagate(False)

        tf = ctk.CTkFrame(sidebar, fg_color='transparent')
        tf.pack(fill='x', padx=10, pady=(10, 5))
        ctk.CTkLabel(tf, text=APP_TITLE, font=ctk.CTkFont(size=18, weight='bold')).pack(side='left')
        ctk.CTkLabel(tf, text="v2", font=ctk.CTkFont(size=10), text_color='gray').pack(side='left', padx=4)

        ctk.CTkButton(sidebar, text="+ 新对话", height=32,
                      command=self._new_conversation).pack(fill='x', padx=10, pady=(5, 5))

        ctk.CTkLabel(sidebar, text="对话历史", font=ctk.CTkFont(size=11),
                     text_color='gray').pack(anchor='w', padx=12, pady=(5, 2))

        self.conv_list_frame = ctk.CTkScrollableFrame(sidebar, fg_color='transparent')
        self.conv_list_frame.pack(fill='both', expand=True, padx=5, pady=0)

        bf = ctk.CTkFrame(sidebar, fg_color='transparent')
        bf.pack(fill='x', padx=10, pady=5)
        ctk.CTkButton(bf, text="切换模型", height=28, width=80,
                      command=self._open_model_select).pack(side='left', padx=(0, 5))
        ctk.CTkButton(bf, text="\u2699 设置", height=28, width=60,
                      command=self._open_settings).pack(side='left')

    def _build_top_bar(self):
        self.top_bar = ctk.CTkFrame(self.root, height=36, corner_radius=0)
        self.top_bar.grid(row=0, column=1, sticky='ew')
        self.top_bar.grid_propagate(False)

        inner = ctk.CTkFrame(self.top_bar, fg_color='transparent')
        inner.pack(fill='x', padx=12, pady=2)

        self.status_indicator = ctk.CTkLabel(inner, text=chr(0x25CF),
                                             font=ctk.CTkFont(size=14), text_color='gray', width=20)
        self.status_indicator.pack(side='left')
        self.status_label = ctk.CTkLabel(inner, text="就绪", font=ctk.CTkFont(size=12))
        self.status_label.pack(side='left', padx=(2, 10))

        self.model_name_label = ctk.CTkLabel(inner, text="", font=ctk.CTkFont(size=11),
                                             text_color='#4caf50')
        self.model_name_label.pack(side='left', padx=(0, 15))

        self.theme_btn = ctk.CTkButton(inner,
                                       text="\u2600" if self.config.get('theme') == 'dark' else "\U0001F315",
                                       width=32, height=28, fg_color='transparent',
                                       command=self._toggle_theme)
        self.theme_btn.pack(side='right', padx=2)

        ctk.CTkButton(inner, text=chr(0x1F5D1), width=32, height=28,
                      fg_color='transparent',
                      command=self._clear_current_chat).pack(side='right', padx=2)

    def _build_chat_area(self):
        self.chat_scroll = ctk.CTkScrollableFrame(self.root, fg_color='transparent')
        self.chat_scroll.grid(row=1, column=1, sticky='nsew')

        self.welcome_label = ctk.CTkLabel(
            self.chat_scroll,
            text="欢迎使用 桌面AI！\n\n"
                 "选择左侧模型即可开始在本地与 AI 对话。\n"
                 "所有数据保存在本地，无需联网（除下载模型外）。",
            font=ctk.CTkFont(size=13), text_color='gray', justify='center', wraplength=400
        )

    def _build_input_area(self):
        input_frame = ctk.CTkFrame(self.root, height=70, corner_radius=0)
        input_frame.grid(row=2, column=1, sticky='ew')
        input_frame.grid_propagate(False)

        inner = ctk.CTkFrame(input_frame, fg_color='transparent')
        inner.pack(fill='both', expand=True, padx=12, pady=6)
        inner.grid_columnconfigure(0, weight=1)

        self.input_textbox = ctk.CTkTextbox(inner, height=50,
                                            font=ctk.CTkFont(size=self.font_size), wrap='word')
        self.input_textbox.grid(row=0, column=0, sticky='ew', padx=(0, 8))

        self.send_btn = ctk.CTkButton(inner, text="发送", width=80, height=50,
                                      font=ctk.CTkFont(size=13, weight='bold'),
                                      command=self._send_message)
        self.send_btn.grid(row=0, column=1)

        self.input_textbox.bind('<Return>', self._on_enter)
        self.input_textbox.bind('<Shift-Return>', lambda e: None)

    def _build_bottom_bar(self):
        self.bottom_bar = ctk.CTkFrame(self.root, height=22, corner_radius=0)
        self.bottom_bar.grid(row=3, column=1, sticky='ew')
        self.stats_label = ctk.CTkLabel(self.bottom_bar, text="",
                                        font=ctk.CTkFont(size=10), text_color='gray')
        self.stats_label.pack(side='right', padx=10)

    # ─── 启动流程 ────────────────────────────────────────

    def _on_startup(self):
        self._refresh_conversation_list()

        last_id = self.config.get('last_conversation_id')
        if last_id and self.conv_manager.load_conversation(last_id):
            self._display_conversation(self.conv_manager.messages)
        else:
            self.conv_manager.new_conversation()
            self.welcome_label.pack(pady=(80, 10))

        selected_id = self.config.get('selected_model_id')
        model_info = get_model_info(self.config, selected_id) if selected_id else None
        model_path = get_models_dir() / model_info['filename'] if model_info else None

        if model_info and model_path and model_path.exists():
            self._load_model_async(model_info)
        else:
            downloaded = get_downloaded_models(self.config)
            if downloaded:
                self.config['selected_model_id'] = downloaded[0]['id']
                save_config(self.config)
                self._load_model_async(downloaded[0])
            else:
                self._prompt_model_selection()

    def _prompt_model_selection(self):
        self.status_indicator.configure(text_color='orange')
        self.status_label.configure(text="请选择模型")
        self.model_name_label.configure(text="")

        dialog = ModelSelectDialog(self.root, self.config)
        self.root.wait_window(dialog)

        if dialog.selected_model_id:
            self.config['selected_model_id'] = dialog.selected_model_id
            save_config(self.config)
            model_info = get_model_info(self.config, dialog.selected_model_id)
            if model_info:
                self._load_model_async(model_info)
        else:
            self.status_indicator.configure(text_color='red')
            self.status_label.configure(text="未选择模型")

    def _load_model_async(self, model_info):
        self.status_indicator.configure(text_color='orange')
        self.status_label.configure(text="正在加载...")
        self.model_name_label.configure(text=model_info['name'])

        def _load():
            success = self.llm.load(
                model_info,
                status_callback=lambda msg: self.root.after(0,
                    lambda m=msg: self.status_label.configure(text=m))
            )
            if success:
                self.root.after(0, self._on_model_ready)
            else:
                self.root.after(0, self._on_model_failed)

        threading.Thread(target=_load, daemon=True).start()

    def _on_model_ready(self):
        self.status_indicator.configure(text_color='#4caf50')
        mi = get_model_info(self.config, self.llm.current_model_id)
        self.status_label.configure(text="就绪")
        self.model_name_label.configure(text=mi['name'] if mi else '模型')
        self._update_stats()

    def _on_model_failed(self):
        self.status_indicator.configure(text_color='red')
        self.status_label.configure(text="加载失败")

    def _reload_model(self):
        if self.llm.is_loaded:
            self.llm.unload()
        selected_id = self.config.get('selected_model_id')
        model_info = get_model_info(self.config, selected_id) if selected_id else None
        if model_info:
            path = get_models_dir() / model_info['filename']
            if path.exists():
                self._load_model_async(model_info)
            else:
                self._prompt_model_selection()
        else:
            self._prompt_model_selection()

    def _open_model_select(self):
        dialog = ModelSelectDialog(self.root, self.config)
        self.root.wait_window(dialog)
        if dialog.selected_model_id:
            old = self.config.get('selected_model_id')
            self.config['selected_model_id'] = dialog.selected_model_id
            save_config(self.config)
            if dialog.selected_model_id != old:
                self._reload_model()

    # ─── 对话管理 ────────────────────────────────────────

    def _new_conversation(self):
        self.conv_manager.new_conversation()
        self._clear_chat_display()
        self.welcome_label.pack(pady=(80, 10))
        self._refresh_conversation_list()
        self._update_stats()

    def _load_conversation(self, conv_id):
        if self.conv_manager.load_conversation(conv_id):
            self._clear_chat_display()
            self._display_conversation(self.conv_manager.messages)
            self.config['last_conversation_id'] = conv_id
            save_config(self.config)
            self._refresh_conversation_list()
            self._update_stats()

    def _delete_conversation(self, conv_id):
        if messagebox.askyesno("确认删除", "确定删除此对话？"):
            self.conv_manager.delete_conversation(conv_id)
            self._refresh_conversation_list()
            if not self.conv_manager.messages:
                self._clear_chat_display()
                self.welcome_label.pack(pady=(80, 10))
            self._update_stats()

    def _clear_current_chat(self):
        if self.is_generating:
            messagebox.showinfo("提示", "请先停止生成")
            return
        if self.conv_manager.message_count == 0:
            return
        if messagebox.askyesno("清空对话", "确定清空当前对话？"):
            self.conv_manager.clear_current()
            self._clear_chat_display()
            self.welcome_label.pack(pady=(80, 10))
            self._refresh_conversation_list()
            self._update_stats()

    def _refresh_conversation_list(self):
        for widget in self.conv_list_frame.winfo_children():
            widget.destroy()

        conversations = self.conv_manager.list_conversations()
        if not conversations:
            ctk.CTkLabel(self.conv_list_frame, text="暂无对话记录",
                         font=ctk.CTkFont(size=11), text_color='gray').pack(pady=20)
            return

        for conv in conversations:
            is_active = conv['id'] == self.conv_manager.current_id
            bg = self.root._apply_appearance_mode(
                ctk.ThemeManager.theme["CTkButton"]["fg_color"]) if is_active else None
            item = ctk.CTkFrame(self.conv_list_frame, fg_color=bg, corner_radius=6)
            item.pack(fill='x', pady=1, padx=2)

            t = conv.get('title', '未命名对话')
            if len(t) > 20:
                t = t[:18] + '...'
            tl = ctk.CTkLabel(item, text=t, font=ctk.CTkFont(size=12), anchor='w')
            tl.pack(anchor='w', padx=8, pady=(4, 0))
            il = ctk.CTkLabel(item, text=f"{conv.get('message_count', 0)} 条消息",
                              font=ctk.CTkFont(size=10), text_color='gray')
            il.pack(anchor='w', padx=8, pady=(0, 4))

            for w in [item, tl, il]:
                w.bind('<Button-1>', lambda e, cid=conv['id']: self._load_conversation(cid))
                w.bind('<Button-3>', lambda e, cid=conv['id']: self._delete_conversation(cid))

    # ─── 消息显示 ────────────────────────────────────────

    def _clear_chat_display(self):
        self.welcome_label.pack_forget()
        for widget in self.chat_scroll.winfo_children():
            if widget != self.welcome_label:
                widget.destroy()
        self._current_ai_textbox = None

    def _display_conversation(self, messages):
        for msg in messages:
            self._add_message_bubble(msg['role'], msg['content'])

    def _add_message_bubble(self, role, content):
        is_user = (role == 'user')
        bg_user = '#0d6efd'
        bg_ai = self.root._apply_appearance_mode(['#e8e8e8', '#2d2d2d'])

        bubble = ctk.CTkFrame(self.chat_scroll, corner_radius=12,
                              fg_color=bg_user if is_user else bg_ai)

        role_text = "你" if is_user else "AI"
        text_color = '#ffffff' if is_user else self.root._apply_appearance_mode(['#333333', '#cccccc'])

        ctk.CTkLabel(bubble, text=role_text,
                     font=ctk.CTkFont(size=11, weight='bold'),
                     text_color=text_color).pack(anchor='w', padx=12, pady=(6, 0))

        if is_user:
            ctk.CTkLabel(bubble, text=content, font=ctk.CTkFont(size=self.font_size),
                         wraplength=450, justify='left',
                         text_color='#ffffff').pack(anchor='w', padx=12, pady=(2, 8))
            bubble.pack(anchor='e', padx=15, pady=4)
        else:
            lines = content.count('\n') + 1
            est_h = max(3, min(lines + 2, 40))
            textbox = ctk.CTkTextbox(bubble, height=est_h * 22,
                                     font=ctk.CTkFont(size=self.font_size),
                                     wrap='word', border_width=0, fg_color='transparent')
            textbox.pack(fill='x', padx=8, pady=(2, 8))
            if content:
                MarkdownRenderer.setup_tags(textbox, self.font_size)
                MarkdownRenderer.render(textbox, content, self.font_size)
            bubble.pack(anchor='w', padx=15, pady=4, fill='x')

        self.chat_scroll._parent_canvas.yview_moveto(1.0)
        self._update_stats()
        self.welcome_label.pack_forget()

    # ─── 发送消息 ────────────────────────────────────────

    def _on_enter(self, event):
        if event.state & 0x1:
            return
        self._send_message()
        return 'break'

    def _send_message(self):
        if self.is_generating:
            return

        user_text = self.input_textbox.get('1.0', 'end-1c').strip()
        if not user_text:
            return

        self.input_textbox.delete('1.0', 'end')

        if not self.llm.is_loaded:
            messagebox.showinfo("提示", "请先选择并加载模型")
            self._open_model_select()
            return

        self._add_message_bubble('user', user_text)
        self.conv_manager.add_message('user', user_text)
        self._refresh_conversation_list()

        self.is_generating = True
        self._stop_event.clear()
        self.send_btn.configure(text="停止", fg_color='#c0392b', hover_color='#e74c3c',
                                command=self._stop_generation)
        self.input_textbox.configure(state='disabled')

        self._create_ai_placeholder()
        self.root.update_idletasks()

        threading.Thread(target=self._generate_response, daemon=True).start()

    def _create_ai_placeholder(self):
        bg = self.root._apply_appearance_mode(['#e8e8e8', '#2d2d2d'])
        bubble = ctk.CTkFrame(self.chat_scroll, corner_radius=12, fg_color=bg)
        ctk.CTkLabel(bubble, text="AI", font=ctk.CTkFont(size=11, weight='bold'),
                     text_color=self.root._apply_appearance_mode(['#333333', '#cccccc'])
                     ).pack(anchor='w', padx=12, pady=(6, 0))

        self._current_ai_textbox = ctk.CTkTextbox(
            bubble, height=80, font=ctk.CTkFont(size=self.font_size),
            wrap='word', border_width=0, fg_color='transparent')
        self._current_ai_textbox.pack(fill='x', padx=8, pady=(2, 8))
        self._current_ai_textbox.configure(state='disabled')

        # 预配置 Markdown 标签
        MarkdownRenderer.setup_tags(self._current_ai_textbox, self.font_size)

        self._current_ai_bubble = bubble
        bubble.pack(anchor='w', padx=15, pady=4, fill='x')
        self.chat_scroll._parent_canvas.yview_moveto(1.0)
        self.welcome_label.pack_forget()

    # ─── 流式输出（v2 优化核心）───────────────────────────

    def _stream_token(self, token):
        """
        v2 优化: streaming 期间仅追加原始文本，不做 Markdown 渲染。
        生成完毕后一次性渲染，性能提升 10 倍以上。
        """
        self._stream_buffer.append(token)
        if not self._stream_scheduled:
            self._stream_scheduled = True
            self.root.after(50, self._flush_stream)

    def _flush_stream(self):
        """批量将 token 以纯文本形式追加到 textbox（不渲染 Markdown）"""
        self._stream_scheduled = False
        if not self._current_ai_textbox or not self._stream_buffer:
            return

        batch = ''.join(self._stream_buffer)
        self._stream_buffer.clear()

        try:
            tb = self._current_ai_textbox
            tb.configure(state='normal')
            tb._textbox.insert('end', batch)
            tb._textbox.see('end')
            tb.configure(state='disabled')
        except Exception:
            pass

    def _generate_response(self):
        full_response = ""
        try:
            system_prompt = self.config.get('system_prompt', "You are a helpful assistant.")
            messages = self.conv_manager.get_context_messages(system_prompt)
            full_response = self.llm.generate(
                messages=messages,
                callback=self._stream_token,
                stop_flag=lambda: self._stop_event.is_set()
            )
        except Exception as e:
            error_text = f"\n\n*[生成出错: {str(e)}]*"
            full_response += error_text
            self.root.after(0, lambda t=error_text: self._append_error(t))
        finally:
            final = full_response
            self.root.after(0, lambda r=final: self._on_generation_done(r))

    def _append_error(self, text):
        if self._current_ai_textbox:
            try:
                self._current_ai_textbox.configure(state='normal')
                self._current_ai_textbox._textbox.insert('end', text)
                self._current_ai_textbox._textbox.see('end')
                self._current_ai_textbox.configure(state='disabled')
            except Exception:
                pass

    def _on_generation_done(self, full_response):
        """
        v2 优化: 生成完毕，执行一次性 Markdown 全量渲染。
        这是整个对话中唯一一次 Markdown 渲染。
        """
        self.is_generating = False
        self._stream_buffer.clear()
        self._stream_scheduled = False

        # 恢复发送按钮
        self.send_btn.configure(text="发送", fg_color='#1f6aa5', hover_color='#144870',
                                command=self._send_message)
        self.input_textbox.configure(state='normal')

        # 添加停止标记
        if self._stop_event.is_set():
            if not full_response.endswith('\n\n*[已停止]*'):
                full_response += '\n\n*[已停止]*'

        # 一次性 Markdown 渲染
        if self._current_ai_textbox:
            try:
                MarkdownRenderer.render(self._current_ai_textbox, full_response, self.font_size)
            except Exception:
                pass

        # 保存对话
        self.conv_manager.add_message('assistant', full_response)
        self.config['last_conversation_id'] = self.conv_manager.current_id
        save_config(self.config)

        self._refresh_conversation_list()
        self._update_stats()
        self._current_ai_textbox = None
        self._current_ai_bubble = None

    def _stop_generation(self):
        self._stop_event.set()
        self.send_btn.configure(text="停止中...", state='disabled')

    # ─── 设置 ────────────────────────────────────────────

    def _toggle_theme(self):
        cur = ctk.get_appearance_mode()
        new = 'light' if cur == 'Dark' else 'dark'
        ctk.set_appearance_mode(new)
        self.config['theme'] = new.lower()
        save_config(self.config)
        self.theme_btn.configure(text="\u2600" if new == 'Dark' else "\U0001F315")

    def _open_settings(self):
        dlg = SettingsDialog(self.root, self.config)
        self.root.wait_window(dlg)

        if dlg.result_config:
            if dlg.result_config.get('theme') != self.config.get('theme'):
                ctk.set_appearance_mode(dlg.result_config['theme'])
                self.theme_btn.configure(
                    text="\u2600" if dlg.result_config['theme'] == 'dark' else "\U0001F315")
            if dlg.result_config.get('font_size') != self.config.get('font_size'):
                self.config['font_size'] = dlg.result_config['font_size']
                self.font_size = dlg.result_config['font_size']
            self.config.update(dlg.result_config)
            save_config(self.config)

    def _update_stats(self):
        mc = self.conv_manager.message_count
        mi = get_model_info(self.config, self.config.get('selected_model_id'))
        mt = f" | {mi['name']}" if mi else ""
        self.stats_label.configure(text=f"消息: {mc}{mt}")

    def _on_close(self):
        if self.is_generating:
            self._stop_event.set()
        if self.conv_manager.current_id:
            self.config['last_conversation_id'] = self.conv_manager.current_id
            save_config(self.config)
        self.root.destroy()

    def run(self):
        self.root.mainloop()
