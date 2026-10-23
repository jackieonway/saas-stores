'use strict';

// Test transport only: Rust records each request and supplies the exact raw
// response bytes, including malformed framing and deliberately delayed bodies.
const net = require('node:net');
const readline = require('node:readline');
const pending = new Map();
let nextId = 1;
const emit = value => process.stdout.write(JSON.stringify(value) + '\n');

const server = net.createServer(socket => {
  let bytes = Buffer.alloc(0);
  socket.on('error', () => {});
  socket.on('data', chunk => {
    bytes = Buffer.concat([bytes, chunk]);
    const end = bytes.indexOf('\r\n\r\n');
    if (end < 0) {
      if (bytes.length > 64 * 1024) socket.destroy();
      return;
    }
    const lines = bytes.subarray(0, end).toString('latin1').split('\r\n');
    const [method, target] = lines.shift().split(' ');
    const headers = {};
    for (const line of lines) {
      const colon = line.indexOf(':');
      if (colon > 0) headers[line.slice(0, colon).toLowerCase()] = line.slice(colon + 1).trim();
    }
    const length = Number(headers['content-length'] || 0);
    if (!Number.isSafeInteger(length) || length < 0 || length > 1024 * 1024) return socket.destroy();
    if (bytes.length < end + 4 + length) return;
    socket.removeAllListeners('data');
    const id = nextId++;
    pending.set(id, socket);
    socket.once('close', () => pending.delete(id));
    emit({ id, request: { method, target, headers, body: Array.from(bytes.subarray(end + 4, end + 4 + length)) } });
  });
});

const input = readline.createInterface({ input: process.stdin });
input.on('line', line => {
  const reply = JSON.parse(line);
  const socket = pending.get(reply.id);
  pending.delete(reply.id);
  if (!socket || socket.destroyed) return;
  if (reply.tail) {
    socket.write(Buffer.from(reply.bytes));
    setTimeout(() => socket.end(Buffer.from(reply.tail.bytes)), reply.tail.delayMs);
  } else socket.end(Buffer.from(reply.bytes));
});
input.on('close', () => process.exit(0));
server.on('error', () => process.exit(1));
server.listen(0, '127.0.0.1', () => emit({ origin: 'http://127.0.0.1:' + server.address().port }));
