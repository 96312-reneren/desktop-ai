import os, sys, zipfile

out, src = sys.argv[1], sys.argv[2]
apk = os.path.join(out, 'base.apk')

with zipfile.ZipFile(apk, 'a', zipfile.ZIP_DEFLATED) as z:
    # classes.dex
    z.write(os.path.join(out, 'classes.dex'), 'classes.dex')
    # native libs
    libdir = os.path.join(out, 'lib', 'arm64-v8a')
    for f in os.listdir(libdir):
        z.write(os.path.join(libdir, f), 'lib/arm64-v8a/' + f)
    # web assets
    assets = os.path.join(src, 'assets')
    for f in os.listdir(assets):
        z.write(os.path.join(assets, f), 'assets/' + f)
print('dex + libs + assets added')
