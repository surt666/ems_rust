import { renderers } from './renderers.mjs';
import { s as serverEntrypointModule } from './chunks/_@astrojs-ssr-adapter_CVUZM6ug.mjs';
import { manifest } from './manifest_DeeXIl6I.mjs';

const serverIslandMap = new Map();;

const _page0 = () => import('./pages/_image.astro.mjs');
const _page1 = () => import('./pages/abenchmark.astro.mjs');
const _page2 = () => import('./pages/alarms.astro.mjs');
const _page3 = () => import('./pages/alarmsetup.astro.mjs');
const _page4 = () => import('./pages/areas.astro.mjs');
const _page5 = () => import('./pages/bbenchmark.astro.mjs');
const _page6 = () => import('./pages/buildingreport.astro.mjs');
const _page7 = () => import('./pages/callaction.astro.mjs');
const _page8 = () => import('./pages/climate.astro.mjs');
const _page9 = () => import('./pages/control.astro.mjs');
const _page10 = () => import('./pages/customreports.astro.mjs');
const _page11 = () => import('./pages/exportdata.astro.mjs');
const _page12 = () => import('./pages/main.astro.mjs');
const _page13 = () => import('./pages/masterdata.astro.mjs');
const _page14 = () => import('./pages/meters.astro.mjs');
const _page15 = () => import('./pages/rianalysis.astro.mjs');
const _page16 = () => import('./pages/rienergymodel.astro.mjs');
const _page17 = () => import('./pages/rimain.astro.mjs');
const _page18 = () => import('./pages/standbyanalysis.astro.mjs');
const _page19 = () => import('./pages/userlist.astro.mjs');
const _page20 = () => import('./pages/index.astro.mjs');
const pageMap = new Map([
    ["node_modules/astro/dist/assets/endpoint/generic.js", _page0],
    ["src/pages/abenchmark.astro", _page1],
    ["src/pages/alarms.astro", _page2],
    ["src/pages/alarmsetup.astro", _page3],
    ["src/pages/areas.astro", _page4],
    ["src/pages/bbenchmark.astro", _page5],
    ["src/pages/buildingreport.astro", _page6],
    ["src/pages/callaction.astro", _page7],
    ["src/pages/climate.astro", _page8],
    ["src/pages/control.astro", _page9],
    ["src/pages/customreports.astro", _page10],
    ["src/pages/exportdata.astro", _page11],
    ["src/pages/main.astro", _page12],
    ["src/pages/masterdata.astro", _page13],
    ["src/pages/meters.astro", _page14],
    ["src/pages/rianalysis.astro", _page15],
    ["src/pages/rienergymodel.astro", _page16],
    ["src/pages/rimain.astro", _page17],
    ["src/pages/standbyanalysis.astro", _page18],
    ["src/pages/userlist.astro", _page19],
    ["src/pages/index.astro", _page20]
]);

const _manifest = Object.assign(manifest, {
    pageMap,
    serverIslandMap,
    renderers,
    actions: () => import('./noop-entrypoint.mjs'),
    middleware: () => import('./_noop-middleware.mjs')
});
const _args = {
    "client": "file:///home/sla/projects/EMS/frontend/.amplify-hosting/static/",
    "server": "file:///home/sla/projects/EMS/frontend/.amplify-hosting/compute/default/",
    "host": false,
    "port": 3000,
    "assets": "_astro"
};

const _start = 'start';
if (Object.prototype.hasOwnProperty.call(serverEntrypointModule, _start)) {
	serverEntrypointModule[_start](_manifest, _args);
}

export { pageMap };
