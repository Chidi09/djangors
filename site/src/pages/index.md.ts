import type { APIRoute } from 'astro';
import markdown from '../content/README.md?raw';
export const GET: APIRoute = async () => new Response(markdown, { headers: { 'Content-Type': 'text/markdown; charset=utf-8' } });
