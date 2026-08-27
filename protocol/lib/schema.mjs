// The schema's own type system. Every rule that can be checked mechanically is checked here, so
// a malformed contract fails at `npm run gen` rather than at `tsc` or at runtime.
import { readFileSync } from 'node:fs';

/** Closed scalar table. `Timestamp` is epoch seconds; `Bytes` is §2.5's `{"b64": "..."}`. */
export const SCALARS = {
  u32: { ts: 'number', rs: 'u32' },
  i32: { ts: 'number', rs: 'i32' },
  i64: { ts: 'number', rs: 'i64' },
  f64: { ts: 'number', rs: 'f64' },
  bool: { ts: 'boolean', rs: 'bool' },
  String: { ts: 'string', rs: 'String' },
  Timestamp: { ts: 'number', rs: 'i64' },
  Bytes: { ts: 'Bytes', rs: 'Bytes' },
};

/** `Name` | `Name?` | `[Name]` | `[Name]?`. Nothing nests; a wire shape that needs more than
 *  this wants a named struct, which is also what makes it greppable from the spec. */
export function parseTypeExpr(expr) {
  if (typeof expr !== 'string') {
    throw new Error(`${JSON.stringify(expr)} is not a type expression`);
  }
  const parts = /^(\[?)([A-Za-z][A-Za-z0-9]*)(\]?)(\??)$/.exec(expr);
  if (!parts) throw new Error(`${JSON.stringify(expr)} is not a type expression`);
  const [, open, base, close, nullable] = parts;
  if ((open === '[') !== (close === ']')) {
    throw new Error(`${JSON.stringify(expr)} is not a type expression`);
  }
  return { base, array: open === '[', nullable: nullable === '?' };
}

/**
 * Types that exist without being declared. `ErrorCode` is synthesised from `errors` and may not
 * be hand-declared, but `ErrorFrame.code` still has to name it — so it must be referenceable
 * while remaining undeclarable. Both halves of that are load-bearing.
 */
const SYNTHESISED = new Set(['ErrorCode']);

function checkRef(schema, expr, where) {
  const { base } = parseTypeExpr(expr);
  if (!SCALARS[base] && !SYNTHESISED.has(base) && !schema.types[base]) {
    throw new Error(`${where}: ${base} is not a declared type or a scalar`);
  }
}

export function validateSchema(schema) {
  if (typeof schema.version !== 'number') throw new Error('schema: version must be a number');
  if (!Array.isArray(schema.errors) || schema.errors.length === 0) {
    throw new Error('schema: errors must be a non-empty array');
  }
  if (schema.types['ErrorCode']) {
    throw new Error('schema: ErrorCode is synthesised from `errors` and must not be declared');
  }

  for (const [name, decl] of Object.entries(schema.types)) {
    if (decl.kind === 'id') {
      if (decl.repr !== 'i64' && decl.repr !== 'String') {
        throw new Error(`type ${name}: id repr must be i64 or String`);
      }
    } else if (decl.kind === 'enum') {
      if (!Array.isArray(decl.variants) || decl.variants.length === 0) {
        throw new Error(`type ${name}: enum needs at least one variant`);
      }
      if (new Set(decl.variants).size !== decl.variants.length) {
        throw new Error(`type ${name}: duplicate enum variant`);
      }
    } else if (decl.kind === 'struct') {
      for (const [field, expr] of Object.entries(decl.fields)) {
        checkRef(schema, expr, `type ${name}.${field}`);
      }
    } else {
      throw new Error(`type ${name}: unknown kind ${JSON.stringify(decl.kind)}`);
    }
  }

  const seen = new Set();
  for (const c of schema.commands) {
    if (seen.has(c.name)) throw new Error(`duplicate command ${c.name}`);
    seen.add(c.name);
    let takesBytes = false;
    for (const [arg, expr] of Object.entries(c.args)) {
      checkRef(schema, expr, `command ${c.name}.${arg}`);
      if (parseTypeExpr(expr).base === 'Bytes') takesBytes = true;
    }
    checkRef(schema, c.returns, `command ${c.name} returns`);
    // §2.4's trust rule, enforced rather than documented.
    if (takesBytes && c.privileged !== true) {
      throw new Error(`command ${c.name} takes Bytes and must be marked privileged`);
    }
    if (c.privileged === true && !takesBytes) {
      throw new Error(`command ${c.name} is privileged but takes no Bytes argument`);
    }
  }

  for (const [topic, events] of Object.entries(schema.topics)) {
    for (const [event, expr] of Object.entries(events)) {
      checkRef(schema, expr, `event ${topic}/${event}`);
    }
  }
}

export function loadSchema(path) {
  const schema = JSON.parse(readFileSync(path, 'utf8'));
  validateSchema(schema);
  return schema;
}
