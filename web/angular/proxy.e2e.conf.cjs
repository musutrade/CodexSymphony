// Keep deterministic browser fixtures away from an existing real A01 deployment.
const port = process.env.E2E_API_PORT ?? '3081';
if (!/^\d+$/.test(port) || Number(port) < 1024 || Number(port) > 65535) {
  throw new Error('E2E_API_PORT must be an unprivileged TCP port');
}
module.exports = {
  '/api/**': { target: `http://127.0.0.1:${port}`, changeOrigin: true, secure: false },
};
