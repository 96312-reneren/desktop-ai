# -*- coding: utf-8 -*-
"""
桌面AI - 模型管理模块
负责模型目录、下载管理、推理和对话历史
"""

import os
import json
import threading
import datetime
from pathlib import Path
import requests


# ─── 路径工具 ───────────────────────────────────────────

def get_app_dir():
    """获取应用数据目录 %APPDATA%/DesktopAI"""
    appdata = os.environ.get('APPDATA', os.path.expanduser('~'))
    app_dir = Path(appdata) / 'DesktopAI'
    app_dir.mkdir(parents=True, exist_ok=True)
    return app_dir


def get_models_dir():
    """获取模型存放目录"""
    models_dir = get_app_dir() / 'models'
    models_dir.mkdir(parents=True, exist_ok=True)
    return models_dir


def get_conversations_dir():
    """获取对话历史目录"""
    conv_dir = get_app_dir() / 'conversations'
    conv_dir.mkdir(parents=True, exist_ok=True)
    return conv_dir


def get_config_path():
    return get_app_dir() / 'config.json'


# ─── 配置管理 ───────────────────────────────────────────

DEFAULT_MODEL_CATALOG = [
    {
        "id": "qwen3-1.7b",
        "name": "Qwen3-1.7B",
        "desc": "超轻量模型，4GB内存即可流畅运行，响应速度极快",
        "size_gb": 1.2,
        "tags": ["轻量", "快速", "低配首选"],
        "url": "https://hf-mirror.com/Qwen/Qwen3-1.7B-GGUF/resolve/main/qwen3-1.7b-instruct-q4_k_m.gguf",
        "filename": "qwen3-1.7b-instruct-q4_k_m.gguf"
    },
    {
        "id": "qwen2.5-3b",
        "name": "Qwen2.5-3B",
        "desc": "轻量均衡模型，兼顾速度与质量，6GB内存推荐",
        "size_gb": 2.0,
        "tags": ["均衡", "推荐"],
        "url": "https://hf-mirror.com/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
        "filename": "qwen2.5-3b-instruct-q4_k_m.gguf"
    },
    {
        "id": "qwen2.5-7b",
        "name": "Qwen2.5-7B",
        "desc": "经典7B模型，综合能力强，8GB内存推荐",
        "size_gb": 4.7,
        "tags": ["经典", "综合"],
        "url": "https://hf-mirror.com/Qwen/Qwen2.5-7B-Instruct-GGUF/resolve/main/qwen2.5-7b-instruct-q4_k_m.gguf",
        "filename": "qwen2.5-7b-instruct-q4_k_m.gguf"
    },
    {
        "id": "qwen2.5-coder-7b",
        "name": "Qwen2.5-Coder-7B",
        "desc": "代码专用模型，擅长编程、代码生成与解释",
        "size_gb": 4.7,
        "tags": ["编程", "代码"],
        "url": "https://hf-mirror.com/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/qwen2.5-coder-7b-instruct-q4_k_m.gguf",
        "filename": "qwen2.5-coder-7b-instruct-q4_k_m.gguf"
    },
    {
        "id": "qwen3-8b",
        "name": "Qwen3-8B",
        "desc": "最新一代8B模型，推理能力更强，10GB内存推荐",
        "size_gb": 5.5,
        "tags": ["最新", "高性能"],
        "url": "https://hf-mirror.com/Qwen/Qwen3-8B-GGUF/resolve/main/qwen3-8b-instruct-q4_k_m.gguf",
        "filename": "qwen3-8b-instruct-q4_k_m.gguf"
    }
]

DEFAULT_CONFIG = {
    "theme": "dark",
    "font_size": 14,
    "n_ctx": 4096,
    "n_threads": "auto",
    "last_conversation_id": None,
    "selected_model_id": None,
    "system_prompt": "You are a helpful assistant.",
    "model_catalog": DEFAULT_MODEL_CATALOG
}


def load_config():
    """加载配置，缺失字段用默认值补全"""
    config = dict(DEFAULT_CONFIG)
    config["model_catalog"] = list(DEFAULT_MODEL_CATALOG)
    config_path = get_config_path()
    if config_path.exists():
        try:
            with open(config_path, 'r', encoding='utf-8') as f:
                user = json.load(f)
                for k in ['theme', 'font_size', 'n_ctx', 'n_threads',
                           'last_conversation_id', 'selected_model_id', 'system_prompt']:
                    if k in user:
                        config[k] = user[k]
                if 'model_catalog' in user:
                    config['model_catalog'] = user['model_catalog']
        except (json.JSONDecodeError, IOError):
            pass
    save_config(config)
    return config


def save_config(config):
    config_path = get_config_path()
    with open(config_path, 'w', encoding='utf-8') as f:
        json.dump(config, f, indent=2, ensure_ascii=False)


def get_model_info(config, model_id):
    """从目录中查找模型信息"""
    for m in config.get('model_catalog', []):
        if m['id'] == model_id:
            return m
    return None


def get_downloaded_models(config):
    """返回已下载的模型列表"""
    models_dir = get_models_dir()
    downloaded = []
    for m in config.get('model_catalog', []):
        path = models_dir / m['filename']
        if path.exists() and path.stat().st_size > 50_000_000:
            downloaded.append(m)
    return downloaded


# ─── 模型下载管理器 ─────────────────────────────────────

class ModelDownloader:
    """模型下载管理器，支持断点续传"""

    def __init__(self):
        self._cancel_flag = False
        self._downloading = False

    @property
    def is_downloading(self):
        return self._downloading

    def cancel(self):
        self._cancel_flag = True

    def download(self, model_info, progress_callback=None, status_callback=None):
        """
        下载指定模型

        model_info: 模型信息字典 (含 url, filename)
        progress_callback(percent): 0-100
        status_callback(text): 状态文本
        返回: True/False
        """
        self._cancel_flag = False
        self._downloading = True

        models_dir = get_models_dir()
        model_path = models_dir / model_info['filename']
        url = model_info['url']

        existing_size = model_path.stat().st_size if model_path.exists() else 0
        headers = {}
        if existing_size > 0:
            headers['Range'] = f'bytes={existing_size}-'

        try:
            if status_callback:
                status_callback("正在连接服务器...")

            response = requests.get(url, stream=True, headers=headers,
                                    timeout=30, allow_redirects=True)

            if response.status_code == 200:
                total_size = int(response.headers.get('content-length', 0))
                mode = 'wb'
                downloaded = 0
                if existing_size > 0 and status_callback:
                    status_callback("从头下载...")
            elif response.status_code == 206:
                content_range = response.headers.get('content-range', '')
                total_size = int(content_range.split('/')[-1]) if '/' in content_range else 0
                mode = 'ab'
                downloaded = existing_size
                if status_callback:
                    status_callback(f"续传中 ({downloaded/1024/1024:.0f} MB / {total_size/1024/1024:.0f} MB)")
            elif response.status_code == 416:
                self._downloading = False
                if status_callback:
                    status_callback("文件已完整")
                return True
            else:
                self._downloading = False
                if status_callback:
                    status_callback(f"服务器错误 (HTTP {response.status_code})")
                return False

            if total_size == 0:
                self._downloading = False
                if status_callback:
                    status_callback("无法获取文件大小，请检查下载地址")
                return False

            if downloaded >= total_size:
                self._downloading = False
                if status_callback:
                    status_callback("已完整下载")
                return True

            with open(model_path, mode) as f:
                for chunk in response.iter_content(chunk_size=1024 * 1024):
                    if self._cancel_flag:
                        if status_callback:
                            status_callback("已取消（进度已保留）")
                        self._downloading = False
                        return False
                    if chunk:
                        f.write(chunk)
                        downloaded += len(chunk)
                        pct = min(int(downloaded / total_size * 100), 100)
                        if progress_callback:
                            progress_callback(pct)
                        if status_callback:
                            status_callback(
                                f"下载中 {pct}% ({downloaded/1024/1024:.0f}/{total_size/1024/1024:.0f} MB)"
                            )

            if status_callback:
                status_callback("下载完成！")
            self._downloading = False
            return True

        except requests.exceptions.ConnectionError:
            self._downloading = False
            if status_callback:
                status_callback("网络连接失败，请检查网络后重试")
            return False
        except requests.exceptions.Timeout:
            self._downloading = False
            if status_callback:
                status_callback("连接超时，请检查网络后重试")
            return False
        except Exception as e:
            self._downloading = False
            if status_callback:
                status_callback(f"出错: {str(e)}")
            return False


# ─── 大语言模型封装 ─────────────────────────────────────

class LLM:
    """大语言模型封装"""

    def __init__(self, config):
        self.config = config
        self.llm = None
        self._loaded = False
        self._loading = False
        self._current_model_id = None

    @property
    def is_loaded(self):
        return self._loaded

    @property
    def is_loading(self):
        return self._loading

    @property
    def current_model_id(self):
        return self._current_model_id

    def load(self, model_info, status_callback=None):
        """
        加载模型到内存

        model_info: 模型信息字典
        status_callback(text): 状态文本
        返回: True/False
        """
        self._loading = True
        try:
            if status_callback:
                status_callback(f"正在加载 {model_info['name']} ...")

            from llama_cpp import Llama

            n_threads = self.config.get('n_threads', 'auto')
            if n_threads == 'auto' or n_threads is None:
                n_threads = os.cpu_count() or 4

            model_path = get_models_dir() / model_info['filename']

            self.llm = Llama(
                model_path=str(model_path),
                n_ctx=self.config.get('n_ctx', 4096),
                n_threads=int(n_threads),
                verbose=False,
                n_gpu_layers=0
            )

            self._loaded = True
            self._loading = False
            self._current_model_id = model_info['id']

            if status_callback:
                status_callback(f"{model_info['name']} 已就绪")

            return True

        except ImportError:
            self._loading = False
            if status_callback:
                status_callback("缺少 llama-cpp-python 库")
            return False
        except Exception as e:
            self._loading = False
            if status_callback:
                status_callback(f"加载失败: {str(e)}")
            return False

    def generate(self, messages, callback=None, stop_flag=None):
        """流式生成回复"""
        if not self._loaded or self.llm is None:
            raise RuntimeError("模型未加载")

        full_response = ""

        try:
            stream = self.llm.create_chat_completion(
                messages=messages,
                stream=True,
                max_tokens=2048,
                temperature=0.7,
                top_p=0.8,
                chat_format="chatml"
            )

            for chunk in stream:
                if stop_flag and stop_flag():
                    break
                if 'choices' in chunk and len(chunk['choices']) > 0:
                    delta = chunk['choices'][0].get('delta', {})
                    content = delta.get('content', '')
                    if content:
                        full_response += content
                        if callback:
                            callback(content)

        except Exception as e:
            error_text = f"\n\n*[生成出错: {str(e)}]*"
            full_response += error_text
            if callback:
                callback(error_text)

        return full_response

    def unload(self):
        if self.llm is not None:
            del self.llm
            self.llm = None
        self._loaded = False
        self._current_model_id = None


# ─── 对话管理器 ─────────────────────────────────────────

class ConversationManager:
    """多轮对话管理"""

    def __init__(self):
        self.conv_dir = get_conversations_dir()
        self.current_id = None
        self.messages = []
        self._max_context = 20

    def list_conversations(self):
        conversations = []
        for f in sorted(self.conv_dir.glob('*.json'), reverse=True):
            try:
                with open(f, 'r', encoding='utf-8') as fh:
                    data = json.load(fh)
                    conversations.append({
                        'id': f.stem,
                        'title': data.get('title', '未命名对话'),
                        'created_at': data.get('created_at', ''),
                        'message_count': len(data.get('messages', []))
                    })
            except (json.JSONDecodeError, IOError):
                pass
        conversations.sort(key=lambda x: x.get('created_at', ''), reverse=True)
        return conversations

    def new_conversation(self):
        conv_id = datetime.datetime.now().strftime('%Y%m%d_%H%M%S_%f')
        self.current_id = conv_id
        self.messages = []
        self._save()
        return conv_id

    def load_conversation(self, conv_id):
        file_path = self.conv_dir / f'{conv_id}.json'
        if not file_path.exists():
            return False
        try:
            with open(file_path, 'r', encoding='utf-8') as f:
                data = json.load(f)
                self.current_id = conv_id
                self.messages = data.get('messages', [])
                return True
        except (json.JSONDecodeError, IOError):
            return False

    def add_message(self, role, content):
        self.messages.append({'role': role, 'content': content})
        self._save()

    def get_context_messages(self, system_prompt=None):
        messages = []
        if system_prompt:
            messages.append({'role': 'system', 'content': system_prompt})
        messages.extend(self.messages[-self._max_context:])
        return messages

    def delete_conversation(self, conv_id):
        file_path = self.conv_dir / f'{conv_id}.json'
        if file_path.exists():
            file_path.unlink()
        if self.current_id == conv_id:
            self.new_conversation()

    def clear_current(self):
        self.messages = []
        if self.current_id:
            self._save()

    def _save(self):
        if not self.current_id:
            return
        file_path = self.conv_dir / f'{self.current_id}.json'
        title = "新对话"
        for msg in self.messages:
            if msg['role'] == 'user':
                title = msg['content'][:50].replace('\n', ' ')
                if len(msg['content']) > 50:
                    title += '...'
                break
        data = {
            'id': self.current_id,
            'title': title,
            'created_at': datetime.datetime.now().isoformat(),
            'messages': self.messages
        }
        with open(file_path, 'w', encoding='utf-8') as f:
            json.dump(data, f, indent=2, ensure_ascii=False)

    @property
    def message_count(self):
        return len(self.messages)
