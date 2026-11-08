// Minimal Discord IPC client for testing Rustcord's Rich Presence server.
// Usage: node scripts/rpc-test.mjs <pipe-index> <client-id> [seconds]
import net from 'node:net';

const [index = '1', clientId, seconds = '20'] = process.argv.slice(2);
if (!clientId) {
  console.error('usage: node scripts/rpc-test.mjs <pipe-index> <client-id> [seconds]');
  process.exit(2);
}

const encode = (op, data) => {
  const body = Buffer.from(JSON.stringify(data));
  const head = Buffer.alloc(8);
  head.writeUInt32LE(op, 0);
  head.writeUInt32LE(body.length, 4);
  return Buffer.concat([head, body]);
};

const sock = net.connect(`\\\\.\\pipe\\discord-ipc-${index}`);
let buf = Buffer.alloc(0);
sock.on('data', (chunk) => {
  buf = Buffer.concat([buf, chunk]);
  while (buf.length >= 8) {
    const op = buf.readUInt32LE(0);
    const len = buf.readUInt32LE(4);
    if (buf.length < 8 + len) break;
    const msg = JSON.parse(buf.subarray(8, 8 + len).toString());
    buf = buf.subarray(8 + len);
    console.log('<-', op, JSON.stringify(msg).slice(0, 300));
    if (msg.evt === 'READY') {
      sock.write(encode(1, {
        cmd: 'SET_ACTIVITY',
        nonce: 'test-1',
        args: {
          pid: process.pid,
          activity: {
            details: 'Rustcord Rich Presence testi',
            state: 'IPC köprüsü çalışıyor',
            timestamps: { start: Math.floor(Date.now() / 1000) },
            buttons: [{ label: 'Rustcord', url: 'https://github.com/' }],
          },
        },
      }));
    }
  }
});
sock.on('connect', () => sock.write(encode(0, { v: 1, client_id: clientId })));
sock.on('error', (e) => { console.error('pipe error:', e.message); process.exit(1); });
setTimeout(() => {
  sock.write(encode(1, { cmd: 'SET_ACTIVITY', nonce: 'test-2', args: { pid: process.pid, activity: null } }));
  setTimeout(() => sock.end(), 500);
}, Number(seconds) * 1000);
