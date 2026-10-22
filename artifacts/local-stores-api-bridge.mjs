// Local development only: forward Windows loopback to WSL's loopback stores API
// through a binary stdio channel. No database credentials enter this bridge.
import net from 'node:net';
import { spawn } from 'node:child_process';

const python = `import os,socket,threading
s=socket.create_connection(('127.0.0.1',3031),timeout=10)
s.settimeout(None)
def write_socket():
 try:
  while True:
   data=os.read(0,65536)
   if not data: break
   s.sendall(data)
 finally:
  try: s.shutdown(socket.SHUT_WR)
  except OSError: pass
threading.Thread(target=write_socket,daemon=True).start()
while True:
 data=s.recv(65536)
 if not data: break
 os.write(1,data)
s.close()
`;
const children = new Set();
const server = net.createServer(socket => {
  const child = spawn('wsl.exe', ['--exec', 'python3', '-u', '-c', python], { windowsHide: true });
  children.add(child);
  socket.pipe(child.stdin);
  child.stdout.pipe(socket);
  child.stderr.resume();
  child.on('error', () => socket.destroy());
  child.on('exit', () => { children.delete(child); socket.end(); });
  child.stdin.on('error', () => socket.destroy());
  socket.on('error', () => child.kill());
  socket.on('close', () => child.kill());
});
server.listen(3031, '127.0.0.1', () => console.log('WSL stores API bridge listening on 127.0.0.1:3031'));
function stop() { server.close(); for (const child of children) child.kill(); }
process.on('SIGINT', stop);
process.on('SIGTERM', stop);

