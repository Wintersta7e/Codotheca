/**
 * The Windows signing hook.
 *
 * The key is not a file: it lives on a hardware token or in a cloud HSM, so the callable build
 * input is a wrapper around the certificate authority's tooling. An unsigned Windows artifact
 * shows a first-launch reputation warning; release notes state that fact instead of engineering
 * around it. This hook must never claim a verification it is not performing.
 */
const { spawnSync } = require('node:child_process');

exports.default = async function sign(configuration) {
  const tool = process.env.CODOTHECA_SIGN_TOOL;
  if (tool === undefined || tool === '') {
    if (process.env.CODOTHECA_REQUIRE_SIGNATURE === '1') {
      throw new Error('sign: CODOTHECA_REQUIRE_SIGNATURE=1 but CODOTHECA_SIGN_TOOL is unset.');
    }
    console.error(
      `sign: UNSIGNED — no CODOTHECA_SIGN_TOOL set. Do not publish ${configuration.path}`,
    );
    return;
  }

  const result = spawnSync(tool, [configuration.path], { stdio: 'inherit' });
  if (result.error) {
    throw new Error(`sign: failed to run the configured signing tool: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(
      `sign: the configured signing tool exited ${String(result.status)} for ${configuration.path}`,
    );
  }
};
