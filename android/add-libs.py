import os, sys, zipfile

out = sys.argv[1]
apk = os.path.join(out, 'base.apk')
libdir = os.path.join(out, 'lib', 'arm64-v8a')
with zipfile.ZipFile(apk, 'a', zipfile.ZIP_DEFLATED) as z:
    for f in os.listdir(libdir):
        z.write(os.path.join(libdir, f), 'lib/arm64-v8a/' + f)
print('libs added:', len(os.listdir(libdir)))
