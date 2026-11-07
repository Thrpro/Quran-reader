import {build} from 'esbuild';
await build({entryPoints:['src/audioCacheWorker.ts'],bundle:true,format:'esm',platform:'browser',target:'es2022',outfile:'dist/audio-cache-worker.js'});
