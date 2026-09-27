#!/usr/bin/env python3
"""Hermetic checks of the actual macOS installer shell and workflow scope."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / '.github/workflows/release.yml').read_text()
EXPRESSION = '${{ matrix.install_dist.run }}'


def step(name):
    return WORKFLOW.split('      - name: ' + name + '\n', 1)[1].split('      - ', 1)[0]


class ReleaseWorkflowTest(unittest.TestCase):
    def test_scope_and_generation_contract(self):
        self.assertEqual((ROOT / 'dist-workspace.toml').read_text().count('allow-dirty = ["ci"]\n'), 1)
        mac = step('Install dist (self-hosted macOS)')
        other = step('Install dist (other runners)')
        self.assertIn("if: ${{ matrix.runner == 'self-hosted' && runner.os == 'macOS' }}", mac)
        self.assertIn("if: ${{ matrix.runner != 'self-hosted' || runner.os != 'macOS' }}", other)
        self.assertIn('run: ' + EXPRESSION, other)
        self.assertEqual(WORKFLOW.count('      - name: Install dist (self-hosted macOS)'), 1)
        self.assertEqual(WORKFLOW.count('      - name: Install dist (other runners)'), 1)
        self.assertIn('runs-on: ${{ matrix.runner }}', WORKFLOW)
        self.assertIn('path: ~/.cargo/bin/dist', WORKFLOW)  # plan cache intact
        self.assertIn('dist build ${{ needs.plan.outputs.tag-flag }}', WORKFLOW)
        self.assertLess(WORKFLOW.index('Install dist (other runners)'), WORKFLOW.index('Build artifacts'))

    def test_concurrent_failure_wrong_dir_and_symlink(self):
        script = step('Install dist (self-hosted macOS)').split('        run: |\n', 1)[1]
        script = '\n'.join(line[10:] for line in script.splitlines())
        script = script.replace(EXPRESSION, 'curl --fake | sh')
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            fakebin = base / 'bin'
            fakebin.mkdir()
            curl = fakebin / 'curl'
            curl.write_text('''#!/bin/sh
[ "${CARGO_DIST_NO_MODIFY_PATH:-}" = 1 ] || exit 23
case "$CARGO_DIST_INSTALL_DIR" in "$RUNNER_TEMP"/shipshape-dist.*) ;; *) exit 24 ;; esac
if [ "${FAIL_INSTALL:-0}" = 1 ]; then exit 22; fi
if [ "${WRONG_DIR:-0}" = 1 ]; then
  echo 'mkdir -p "$HOME/.cargo/bin"; touch "$HOME/.cargo/bin/dist"'
elif [ "${SYMLINK:-0}" = 1 ]; then
  echo 'mkdir -p "$CARGO_DIST_INSTALL_DIR/bin"; ln -s "$HOME/.cargo/bin/dist" "$CARGO_DIST_INSTALL_DIR/bin/dist"'
else
  printf '%s\\n' 'mkdir -p "$CARGO_DIST_INSTALL_DIR/bin"' \\
    'printf "#!/bin/sh\\\\nexit 0\\\\n" > "$CARGO_DIST_INSTALL_DIR/bin/dist"' \\
    'chmod +x "$CARGO_DIST_INSTALL_DIR/bin/dist"'
fi
''')
            curl.chmod(0o755)
            home = base / 'home'
            (home / '.cargo/bin').mkdir(parents=True)
            existing = home / '.cargo/bin/cargo-dist'
            existing.write_text('unchanged')
            env = dict(os.environ, HOME=str(home), RUNNER_TEMP=str(base),
                       PATH=f'{fakebin}:{os.environ["PATH"]}')
            outputs = [base / f'path-{i}' for i in range(5)]
            for output in outputs:
                output.touch()
            procs = [subprocess.Popen(['bash', '-c', script],
                     env=dict(env, GITHUB_PATH=str(output),
                              FAIL_INSTALL=str(int(i == 2)), WRONG_DIR=str(int(i == 3)),
                              SYMLINK=str(int(i == 4))),
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                     for i, output in enumerate(outputs)]
            results = [p.communicate() for p in procs]
            self.assertEqual([p.returncode for p in procs], [0, 0, 22, 1, 1], results)
            roots = [o.read_text().strip() for o in outputs]
            self.assertEqual(roots[2:], ['', '', ''])
            self.assertEqual(len(set(roots[:2])), 2)
            for root in roots[:2]:
                self.assertTrue(root.startswith(str(base / 'shipshape-dist.')))
                self.assertTrue((Path(root) / 'dist').is_file())
                resolved = subprocess.check_output(['bash', '-c', 'command -v dist'],
                     env=dict(env, PATH=f'{root}:{env["PATH"]}'), text=True).strip()
                self.assertEqual(resolved, str(Path(root) / 'dist'))
            self.assertEqual(existing.read_text(), 'unchanged')
            self.assertTrue((home / '.cargo/bin/dist').exists())  # detected, not erased
            missing = subprocess.run(['bash', '-c', script], env=dict(env, RUNNER_TEMP='',
                                     GITHUB_PATH=str(outputs[2])), capture_output=True)
            self.assertNotEqual(missing.returncode, 0)
            self.assertEqual(outputs[2].read_text(), '')


if __name__ == '__main__':
    unittest.main()
