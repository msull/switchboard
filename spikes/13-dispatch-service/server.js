// The way react-scripts starts: a parent that forks the dev server and waits.
const { spawn } = require('child_process');
const child = spawn(process.execPath, ['-e', `
  require('http').createServer((q, s) => { s.end('ok'); }).listen(process.env.PORT, process.env.HOST || undefined);
`], { stdio: 'inherit' });
child.on('exit', (c) => process.exit(c ?? 1));
