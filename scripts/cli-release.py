"""Native release verification and portable CLI archives. Python is CI-only."""
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
VERSION = json.loads((ROOT / 'package.json').read_text())['version']


def run(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True).strip()


def result(binary, *args):
    envelope = json.loads(run(binary, *args))
    assert envelope['ok'], envelope
    return envelope['result']


def verify(binary, target):
    assert run(binary, '--version') == f'downloader-cli {VERSION}'
    native = result(binary, 'tools', 'platform')
    expected_os = 'macos' if 'apple' in target else 'windows' if 'windows' in target else 'linux'
    assert native['os'] == expected_os, native
    assert native['arch'] == target.split('-')[0], native
    assert native['ytDlpAsset'] and native['ffmpegAsset'], native
    assert result(binary, 'schema')['schemaVersion'] == 1
    if expected_os == 'macos':
        arch = 'arm64' if target.startswith('aarch64') else 'x86_64'
        assert run('lipo', '-archs', binary) == arch
        deps = run('otool', '-L', binary)
        assert '/opt/homebrew/' not in deps and '/usr/local/' not in deps, deps
    elif expected_os == 'linux':
        deps = run('ldd', binary)
        assert all(term not in deps.lower() for term in ['webkit', 'gtk', 'not found']), deps
    print(f'Native CLI verified: {target}, {platform.platform()}', flush=True)


def main():
    operation, target = sys.argv[1:]
    name = 'downloader-cli.exe' if 'windows' in target else 'downloader-cli'
    binary = ROOT / 'src-tauri' / 'target' / target / 'release' / name
    if operation == 'verify':
        verify(binary, target)
        result(binary, 'tools', 'install', 'all')
        status = result(binary, 'tools', 'status')
        paths = set()
        for tool in status:
            assert tool['status'] == 'installed', tool
            paths.add(str(Path(tool['path']).parent))
        with open(os.environ['GITHUB_PATH'], 'a') as handle:
            handle.write('\n'.join(sorted(paths)) + '\n')
        print('Native yt-dlp, FFmpeg, and ffprobe installed and verified.', flush=True)
    elif operation == 'package':
        dist = ROOT / 'cli-dist'
        dist.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary) / 'stage'
            stage.mkdir()
            shutil.copy2(binary, stage / name)
            for source in ['docs/cli.md', 'examples/adminhub-downloader.mjs']:
                destination = stage / source
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / source, destination)
            if 'apple' in target:
                subprocess.run(['codesign', '--force', '--sign', '-', str(stage / name)], check=True)
                subprocess.run(['codesign', '--verify', '--strict', str(stage / name)], check=True)
            base = f'downloader-cli-{VERSION}-{target}'
            if 'windows' in target:
                archive = dist / f'{base}.zip'
                with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as package:
                    for path in sorted(stage.rglob('*')):
                        if path.is_file():
                            package.write(path, path.relative_to(stage))
            else:
                archive = dist / f'{base}.tar.gz'
                with tarfile.open(archive, 'w:gz') as package:
                    for path in sorted(stage.iterdir()):
                        package.add(path, arcname=path.name)
            extracted = Path(temporary) / 'extracted'
            shutil.unpack_archive(archive, extracted)
            verify(extracted / name, target)
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            (dist / f'{archive.name}.sha256').write_text(f'{digest}  {archive.name}\n')
            print(f'Packaged and verified {archive.name}', flush=True)
    else:
        raise ValueError(operation)


if __name__ == '__main__':
    main()
