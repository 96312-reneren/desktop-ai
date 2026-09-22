"""Keystore management for the zero-Gradle APK pipeline.

Ensures a signing keystore with a *random* password exists outside the
repo/build dirs. The password is never hardcoded in build scripts; it is
stored in a local file next to the keystore (which lives outside the
repository, so nothing secret is committed).

Usage:
  python keystore-mgr.py ensure   # create keystore + random password (rotates
                                  # legacy hardcoded 'desktopai123' passwords)
  python keystore-mgr.py pass     # print the current password (stdout)
"""

import os
import secrets
import subprocess
import sys

HOME = os.path.expanduser("~")
DIR = os.path.join(HOME, ".desktopai-android")
KEYSTORE = os.path.join(DIR, "desktopai.keystore")
PASSFILE = os.path.join(DIR, "keystore.pass")
ALIAS = "desktopai"
LEGACY_PASS = "desktopai123"  # rotated away on first run of 'ensure'


def keytool(args):
    java = os.path.join(os.environ.get("JAVA_HOME", ""), "bin", "keytool.exe")
    if not os.path.isfile(java):
        java = "keytool"
    return subprocess.run([java] + args, capture_output=True, text=True)


def generate_pass():
    return secrets.token_urlsafe(24)


def ensure():
    os.makedirs(DIR, exist_ok=True)
    if not os.path.exists(KEYSTORE):
        # Fresh keystore with a random password.
        pwd = generate_pass()
        r = keytool([
            "-genkeypair", "-v", "-keystore", KEYSTORE, "-alias", ALIAS,
            "-keyalg", "RSA", "-keysize", "2048", "-validity", "10000",
            "-storepass", pwd, "-keypass", pwd, "-dname", "CN=DesktopAI",
        ])
        if r.returncode != 0:
            sys.exit("keytool failed: " + r.stderr)
        write_pass(pwd)
        print("keystore created with random password")
        return
    # Keystore exists; rotate a legacy hardcoded password if the pass file
    # is missing or stale (first run of this tool).
    pwd = generate_pass()
    if os.path.exists(PASSFILE):
        with open(PASSFILE) as f:
            existing = f.read().strip()
        if existing:
            print("password already managed")
            return
    # Try the legacy password; if that fails the password is already random
    # and we just persist it.
    r = keytool([
        "-storepasswd", "-keystore", KEYSTORE,
        "-storepass", LEGACY_PASS, "-new", pwd,
    ])
    if r.returncode != 0:
        # Not the legacy password — the keystore password is already random.
        print("keystore password is not the legacy one; leaving it as-is")
        return
    keytool([
        "-keypasswd", "-keystore", KEYSTORE, "-alias", ALIAS,
        "-storepass", pwd, "-keypass", LEGACY_PASS, "-new", pwd,
    ])
    write_pass(pwd)
    print("legacy password rotated to a random one")


def write_pass(pwd):
    fd = os.open(PASSFILE, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(pwd)


def show_pass():
    if not os.path.exists(PASSFILE):
        sys.exit("no password file — run 'ensure' first")
    with open(PASSFILE) as f:
        sys.stdout.write(f.read().strip())


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "ensure"
    if cmd == "ensure":
        ensure()
    elif cmd == "pass":
        show_pass()
    else:
        sys.exit("unknown command: " + cmd)
