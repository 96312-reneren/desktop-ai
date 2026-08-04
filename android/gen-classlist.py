import glob, os, sys

classes_dir, out_list = sys.argv[1], sys.argv[2]
with open(out_list, 'w', encoding='utf-8') as f:
    for p in glob.glob(os.path.join(classes_dir, '**', '*.class'), recursive=True):
        f.write(p + '\n')
print('classlist written')
