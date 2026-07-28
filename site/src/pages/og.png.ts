import type { APIRoute } from 'astro';
import satori from 'satori';
import { Resvg } from '@resvg/resvg-js';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const asset = (relative: string) => join(process.cwd(), relative);

const logoDataUri = `data:image/png;base64,${readFileSync(asset('public/android-chrome-512x512.png')).toString('base64')}`;
const fontRegular = readFileSync(asset('src/assets/fonts/LiberationSans-Regular.ttf'));
const fontBold = readFileSync(asset('src/assets/fonts/LiberationSans-Bold.ttf'));

export const GET: APIRoute = async () => {
  const svg = await satori(
    {
      type: 'div',
      props: {
        style: {
          width: '1200px',
          height: '630px',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: '#201e1e',
        },
        children: [
          {
            type: 'img',
            props: { src: logoDataUri, width: 180, height: 180, style: { marginBottom: '32px' } },
          },
          {
            type: 'div',
            props: {
              style: { display: 'flex', fontSize: 96, fontWeight: 700, color: '#f5f1e7' },
              children: 'Djangors',
            },
          },
          {
            type: 'div',
            props: {
              style: { display: 'flex', fontSize: 40, color: '#f27822', marginTop: '16px' },
              children: 'The Django of Rust',
            },
          },
          {
            type: 'div',
            props: {
              style: { display: 'flex', fontSize: 26, color: '#f5f1e7', opacity: 0.7, marginTop: '28px' },
              children: 'Batteries-included web framework · ORM · admin · auth · REST · a single static binary',
            },
          },
        ],
      },
    },
    {
      width: 1200,
      height: 630,
      fonts: [
        { name: 'Liberation Sans', data: fontRegular, weight: 400, style: 'normal' },
        { name: 'Liberation Sans', data: fontBold, weight: 700, style: 'normal' },
      ],
    },
  );

  const png = new Resvg(svg, { fitTo: { mode: 'width', value: 1200 } }).render().asPng();
  return new Response(png, { headers: { 'Content-Type': 'image/png' } });
};
