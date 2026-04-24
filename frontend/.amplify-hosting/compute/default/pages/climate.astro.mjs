import { e as createComponent, f as createAstro, k as renderComponent, r as renderTemplate, m as maybeRenderHead, l as renderScript } from '../chunks/astro/server_bfQdwCeF.mjs';
import { $ as $$Layout } from '../chunks/Layout_B-tuoUaj.mjs';
/* empty css                                   */
export { renderers } from '../renderers.mjs';

const $$Astro = createAstro();
const $$Climate = createComponent(async ($$result, $$props, $$slots) => {
  const Astro2 = $$result.createAstro($$Astro, $$props, $$slots);
  Astro2.self = $$Climate;
  const { dashboardId } = Astro2.props;
  return renderTemplate`${renderComponent($$result, "Layout", $$Layout, { "title": "Klimaregnskab", "data-astro-cid-3bwlwyxz": true }, { "default": async ($$result2) => renderTemplate` ${maybeRenderHead()}<div class="p-8" data-astro-cid-3bwlwyxz> <h1 class="text-3xl font-bold mb-6" data-astro-cid-3bwlwyxz>CO2-regnskab og klimapåvirkning</h1> <div id="dashboard-container" data-astro-cid-3bwlwyxz></div> <!-- Client-side script --> ${renderScript($$result2, "/home/sla/projects/EMS/frontend/src/pages/climate.astro?astro&type=script&index=0&lang.ts")}  </div> ` })}`;
}, "/home/sla/projects/EMS/frontend/src/pages/climate.astro", void 0);

const $$file = "/home/sla/projects/EMS/frontend/src/pages/climate.astro";
const $$url = "/climate";

const _page = /*#__PURE__*/Object.freeze(/*#__PURE__*/Object.defineProperty({
	__proto__: null,
	default: $$Climate,
	file: $$file,
	url: $$url
}, Symbol.toStringTag, { value: 'Module' }));

const page = () => _page;

export { page };
