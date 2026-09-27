#!/usr/bin/env python3
"""Check/write pristine cargo-dist 0.33.0 CI plus Shipshape's local macOS overlay.

Native dist CI drift checking skips allow-dirty = ["ci"]. Regenerate in a
throwaway copy without that exception; never generate over the live workflow.
"""
import argparse
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = Path('.github/workflows/release.yml')
ORIGINAL = """      - name: Install dist
        run: ${{ matrix.install_dist.run }}
      # Get the dist-manifest"""
REPLACEMENT = """      # Repo-local exception: the macOS ARM64 artifact row uses a persistent
      # self-hosted runner. Hosted rows keep the generated matrix installer.
      - name: Install dist (self-hosted macOS)
        if: ${{ matrix.runner == 'self-hosted' && runner.os == 'macOS' }}
        shell: bash
        run: |
          set -euo pipefail
          : "${RUNNER_TEMP:?RUNNER_TEMP must be set for isolated dist install}"
          export CARGO_DIST_INSTALL_DIR="$(mktemp -d "$RUNNER_TEMP/shipshape-dist.XXXXXXXX")"
          export CARGO_DIST_NO_MODIFY_PATH=1
          ${{ matrix.install_dist.run }}
          export PATH="$CARGO_DIST_INSTALL_DIR/bin:$PATH"
          # Fail closed on an old PATH entry, missing binary, or symlink escape.
          test "$(command -v dist)" = "$CARGO_DIST_INSTALL_DIR/bin/dist"
          test -x "$CARGO_DIST_INSTALL_DIR/bin/dist"
          test "$(realpath "$(command -v dist)")" = "$CARGO_DIST_INSTALL_DIR/bin/dist"
          echo "$CARGO_DIST_INSTALL_DIR/bin" >> "$GITHUB_PATH"
      - name: Install dist (other runners)
        if: ${{ matrix.runner != 'self-hosted' || runner.os != 'macOS' }}
        run: ${{ matrix.install_dist.run }}
      # Get the dist-manifest"""


def generate(dist):
    # Cargo metadata walks to the workspace root. Use a snapshot outside it;
    # copy tracked HEAD and replace only the live dist config (also before commit).
    with tempfile.TemporaryDirectory(prefix='shipshape-dist-generate-') as tmp:
        with subprocess.Popen(['git', 'archive', 'HEAD'], cwd=ROOT, stdout=subprocess.PIPE) as git:
            subprocess.run(['tar', '-xf', '-', '-C', tmp], stdin=git.stdout, check=True)
            git.stdout.close()
            if git.wait() != 0:
                raise RuntimeError('git archive failed')
        config = (ROOT / 'dist-workspace.toml').read_text()
        marker = 'allow-dirty = ["ci"]\n'
        if config.count(marker) != 1:
            raise RuntimeError('expected exactly one narrow CI allow-dirty exception')
        (Path(tmp) / 'dist-workspace.toml').write_text(config.replace(marker, ''))
        subprocess.run([dist, 'generate', '--mode', 'ci'], cwd=tmp, check=True)
        subprocess.run([dist, 'generate', '--mode', 'ci', '--check'], cwd=tmp, check=True)
        generated = (Path(tmp) / WORKFLOW).read_text()
        if generated.count(ORIGINAL) != 1:
            raise RuntimeError('cargo-dist local installer template changed; review overlay')
        return generated.replace(ORIGINAL, REPLACEMENT)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    operation = parser.add_mutually_exclusive_group(required=True)
    operation.add_argument('--check', action='store_true')
    operation.add_argument('--write', action='store_true')
    parser.add_argument('--dist', default='dist', help='path to pinned cargo-dist 0.33.0')
    args = parser.parse_args()
    dist = shutil.which(args.dist)
    if not dist:
        parser.error(f'dist binary not found: {args.dist}')
    version = subprocess.check_output([dist, '--version'], text=True).strip()
    if version != 'cargo-dist 0.33.0':
        parser.error(f'expected cargo-dist 0.33.0, got {version}')
    expected = generate(str(Path(dist).resolve()))
    path = ROOT / WORKFLOW
    if args.write:
        path.write_text(expected)
        print(f'generated {WORKFLOW}')
    elif path.read_text() != expected:
        raise SystemExit(f'{WORKFLOW} differs from pristine generation + macOS overlay; run --write')
    else:
        print(f'{WORKFLOW}: pristine generation + macOS overlay match')


if __name__ == '__main__':
    main()
