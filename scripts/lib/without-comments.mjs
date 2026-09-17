/**
 * Comments blanked to spaces, so every line number and offset is unchanged.
 *
 * **One owner, because two gates need it and a second copy would drift** (R12's shape).
 * `check-destructive-tokens.mjs` scans for banned rendered strings and
 * `app/test/uninstallAcceptance.test.ts` scans for override wording and relocate call sites; both
 * are rules about what the product *does*, and a comment **naming** a rule must not trip it. That
 * is not a convenience: the uninstall acceptance suite failed against its own neighbours for
 * saying, in prose, that they ship no `RECLAIM SPACE` and no *uninstall anyway*.
 *
 * The `[^:]` guard on the line comment is what keeps `https://` from eating the rest of a line.
 *
 * @param {string} source
 * @returns {string}
 */
export function withoutComments(source) {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\r\n]/g, ' '))
    .replace(/(^|[^:])\/\/[^\r\n]*/g, (comment, lead) => {
      return lead + ' '.repeat(comment.length - lead.length);
    });
}
