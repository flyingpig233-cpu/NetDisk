import { defineConfig, loadEnv } from 'vite';

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), '');
  return {
    server: {
      host: '0.0.0.0',
      port: 5173,
      proxy: { '/api': { target: env.VITE_API_TARGET || 'http://127.0.0.1:3000', changeOrigin: true, rewrite: path => path.replace(/^\/api/, '') } },
    },
  };
});
