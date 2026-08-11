'use strict';

function maskJadrenSource(source) {
  let result = '';
  let quote = '';
  let escaped = false;
  let lineComment = false;
  let blockComment = false;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index];
    const next = source[index + 1];
    if (lineComment) {
      result += character === '\n' ? '\n' : ' ';
      if (character === '\n') lineComment = false;
      continue;
    }
    if (blockComment) {
      result += character === '\n' ? '\n' : ' ';
      if (character === '*' && next === '/') {
        result += ' ';
        index += 1;
        blockComment = false;
      }
      continue;
    }
    if (quote) {
      result += character === '\n' ? '\n' : ' ';
      if (escaped) escaped = false;
      else if (character === '\\') escaped = true;
      else if (character === quote) quote = '';
      continue;
    }
    if (character === '/' && next === '/') {
      result += '  ';
      index += 1;
      lineComment = true;
      continue;
    }
    if (character === '/' && next === '*') {
      result += '  ';
      index += 1;
      blockComment = true;
      continue;
    }
    if (character === '"' || character === "'") {
      result += ' ';
      quote = character;
      continue;
    }
    result += character;
  }
  return result;
}

function matchingDelimiter(source, openIndex, openCharacter, closeCharacter) {
  let depth = 0;
  for (let index = openIndex; index < source.length; index += 1) {
    if (source[index] === openCharacter) depth += 1;
    else if (source[index] === closeCharacter) {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return source.length - 1;
}

function splitTopLevel(source, delimiter = ',') {
  const parts = [];
  let start = 0;
  let angle = 0;
  let paren = 0;
  let square = 0;
  for (let index = 0; index < source.length; index += 1) {
    const character = source[index];
    if (character === '<') angle += 1;
    else if (character === '>' && angle > 0) angle -= 1;
    else if (character === '(') paren += 1;
    else if (character === ')' && paren > 0) paren -= 1;
    else if (character === '[') square += 1;
    else if (character === ']' && square > 0) square -= 1;
    else if (character === delimiter && angle === 0 && paren === 0 && square === 0) {
      parts.push(source.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(source.slice(start).trim());
  return parts.filter(Boolean);
}

function splitDeclarationEntries(source) {
  const entries = [];
  for (const commaPart of splitTopLevel(source)) {
    let start = 0;
    let angle = 0;
    let paren = 0;
    for (let index = 0; index < commaPart.length; index += 1) {
      const character = commaPart[index];
      if (character === '<') angle += 1;
      else if (character === '>' && angle > 0) angle -= 1;
      else if (character === '(') paren += 1;
      else if (character === ')' && paren > 0) paren -= 1;
      else if (character === '\n' && angle === 0 && paren === 0) {
        const entry = commaPart.slice(start, index).trim();
        if (entry) entries.push(entry);
        start = index + 1;
      }
    }
    const entry = commaPart.slice(start).trim();
    if (entry) entries.push(entry);
  }
  return entries;
}

function parseParameters(source) {
  return splitTopLevel(source).map((raw) => {
    const cleaned = raw.replace(/^\s*(?:pub|mut|read|write|shared|owned)\s+/, '').trim();
    const match = cleaned.match(/^([A-Za-z_][A-Za-z0-9_]*)(?:\s*:\s*(.+))?$/);
    if (!match) return { name: cleaned, type: '' };
    return { name: match[1], type: match[2] ? match[2].trim() : '' };
  }).filter((parameter) => parameter.name);
}

function parseFunctions(source, masked) {
  const functions = [];
  const pattern = /\b(?:pub\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s*<[^>{}]*>)?\s*\(/g;
  let match;
  while ((match = pattern.exec(masked)) !== null) {
    const open = masked.indexOf('(', match.index);
    const close = matchingDelimiter(masked, open, '(', ')');
    const paramsSource = source.slice(open + 1, close);
    const nextBrace = masked.indexOf('{', close + 1);
    const nextLine = masked.indexOf('\n', close + 1);
    const headerEnd = nextBrace >= 0 && (nextLine < 0 || nextBrace < nextLine) ? nextBrace : (nextLine >= 0 ? nextLine : source.length);
    const header = masked.slice(close + 1, headerEnd);
    const returnMatch = header.match(/->\s*([^;{]+?)\s*$/);
    const returnType = returnMatch ? returnMatch[1].trim() : 'Unit';
    const bodyStart = nextBrace >= 0 ? nextBrace : close;
    const bodyEnd = nextBrace >= 0 ? matchingDelimiter(masked, nextBrace, '{', '}') : close;
    const nameStart = match.index + match[0].indexOf(match[1]);
    const parameters = parseParameters(paramsSource);
    functions.push({
      name: match[1],
      nameStart,
      parameters,
      returnType,
      bodyStart,
      bodyEnd,
      signature: `fn ${match[1]}(${parameters.map((parameter) => `${parameter.name}${parameter.type ? `: ${parameter.type}` : ''}`).join(', ')}) -> ${returnType}`,
    });
  }
  return functions;
}

function parseTypes(source, masked) {
  const types = new Map();
  const pattern = /\b(?:pub\s+)?(struct|enum)\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{/g;
  let match;
  while ((match = pattern.exec(masked)) !== null) {
    const open = masked.indexOf('{', match.index);
    const close = matchingDelimiter(masked, open, '{', '}');
    const body = source.slice(open + 1, close);
    const maskedBody = masked.slice(open + 1, close);
    const entries = splitDeclarationEntries(maskedBody);
    const members = [];
    if (match[1] === 'struct') {
      for (const entry of entries) {
        const field = entry.match(/^(?:pub\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(.+)$/s);
        if (field) members.push({ name: field[1], type: field[2].trim(), kind: 'field' });
      }
    } else {
      for (const entry of entries) {
        const variant = entry.match(/^(?:pub\s+)?([A-Za-z_][A-Za-z0-9_]*)/);
        if (variant) members.push({ name: variant[1], type: match[2], kind: 'enum' });
      }
    }
    types.set(match[2], { name: match[2], kind: match[1], members });
  }
  return types;
}

function functionForOffset(functions, offset) {
  return functions
    .filter((candidate) => candidate.bodyStart <= offset && offset <= candidate.bodyEnd)
    .sort((left, right) => (left.bodyEnd - left.bodyStart) - (right.bodyEnd - right.bodyStart))[0];
}

function inferExpressionType(expression, types) {
  const value = expression.trim();
  if (/^(?:true|false)\b/.test(value)) return 'Bool';
  if (/^[-+]?(?:\d+\.\d*|\d*\.\d+)\b/.test(value)) return 'Float64';
  if (/^[-+]?\d+\b/.test(value)) return 'Int32';
  if (/^["']/.test(value)) return 'String';
  const constructor = value.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*\{/);
  if (constructor && types.has(constructor[1])) return constructor[1];
  const variant = value.match(/^([A-Za-z_][A-Za-z0-9_]*)\s*\./);
  if (variant && types.has(variant[1])) return variant[1];
  return '';
}

function parseBindings(source, masked, functions, types, offset) {
  const bindings = new Map();
  const pattern = /\b(?:let|var|const)\s+([A-Za-z_][A-Za-z0-9_]*)(?:\s*:\s*([^=;\n{}]+))?\s*(?:=\s*([^;\n{}]+))?/g;
  let match;
  while ((match = pattern.exec(masked)) !== null) {
    if (match.index >= offset) continue;
    const owner = functionForOffset(functions, match.index);
    const cursorOwner = functionForOffset(functions, offset);
    if (owner !== cursorOwner) continue;
    const type = match[2] ? match[2].trim() : inferExpressionType(match[3] || '', types);
    bindings.set(match[1], {
      name: match[1],
      type,
      declaration: source.slice(match.index, pattern.lastIndex).trim(),
    });
  }
  return bindings;
}

function buildLocalModel(source, offset = source.length) {
  const masked = maskJadrenSource(source);
  const functions = parseFunctions(source, masked);
  const types = parseTypes(source, masked);
  const bindings = parseBindings(source, masked, functions, types, offset);
  return { source, masked, offset, functions, types, bindings };
}

function resolveType(model, expression) {
  const path = expression.replace(/\s+/g, '').split('.').filter(Boolean);
  if (path.length === 0) return '';
  let typeName = model.bindings.get(path[0])?.type || (model.types.has(path[0]) ? path[0] : '');
  for (const memberName of path.slice(1)) {
    const type = model.types.get(typeName);
    const member = type?.members.find((candidate) => candidate.name === memberName);
    if (!member) return '';
    typeName = member.type;
  }
  return typeName;
}

function dotContext(source, offset) {
  const lineStart = source.lastIndexOf('\n', Math.max(0, offset - 1)) + 1;
  const before = source.slice(lineStart, offset);
  const match = before.match(/([A-Za-z_][A-Za-z0-9_]*(?:\s*\.\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)?$/);
  if (!match) return undefined;
  const memberPrefix = match[2] || '';
  return {
    receiver: match[1].replace(/\s+/g, ''),
    memberPrefix,
    memberStart: offset - memberPrefix.length,
  };
}

function membersForDot(model, context) {
  const typeName = resolveType(model, context.receiver);
  const type = model.types.get(typeName);
  if (!type) return [];
  return type.members.map((member) => ({ ...member, owner: type.name }));
}

function activeCall(source, offset) {
  const masked = maskJadrenSource(source.slice(0, offset));
  const stack = [];
  for (let index = 0; index < masked.length; index += 1) {
    const character = masked[index];
    if (character === '(') {
      const before = masked.slice(0, index);
      const name = before.match(/([A-Za-z_][A-Za-z0-9_]*(?:\s*\.\s*[A-Za-z_][A-Za-z0-9_]*)*)\s*$/)?.[1];
      stack.push({ name: name ? name.replace(/\s+/g, '') : '', activeParameter: 0 });
    } else if (character === ')') {
      stack.pop();
    } else if (character === ',' && stack.length > 0) {
      stack[stack.length - 1].activeParameter += 1;
    }
  }
  return stack[stack.length - 1];
}

module.exports = {
  activeCall,
  buildLocalModel,
  dotContext,
  functionForOffset,
  membersForDot,
  resolveType,
};
