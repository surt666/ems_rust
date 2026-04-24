import { e as createComponent, k as renderComponent, r as renderTemplate, m as maybeRenderHead } from '../chunks/astro/server_bfQdwCeF.mjs';
import { $ as $$Layout } from '../chunks/Layout_B-tuoUaj.mjs';
export { renderers } from '../renderers.mjs';

const $$Customreports = createComponent(($$result, $$props, $$slots) => {
  return renderTemplate`${renderComponent($$result, "Layout", $$Layout, { "title": "Brugertilpassede Rapporter" }, { "default": ($$result2) => renderTemplate` ${maybeRenderHead()}<div class="p-8"> <h1 class="text-3xl font-bold mb-6">Brugertilpassede Rapporter</h1> <p class="text-gray-600">Opret og administrér tilpassede energirapporter.</p> </div> ` })}`;
}, "/home/sla/projects/EMS/frontend/src/pages/customreports.astro", void 0);

const $$file = "/home/sla/projects/EMS/frontend/src/pages/customreports.astro";
const $$url = "/customreports";

const _page = /*#__PURE__*/Object.freeze(/*#__PURE__*/Object.defineProperty({
	__proto__: null,
	default: $$Customreports,
	file: $$file,
	url: $$url
}, Symbol.toStringTag, { value: 'Module' }));

const page = () => _page;

export { page };
