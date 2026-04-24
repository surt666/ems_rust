import { e as createComponent, f as createAstro, k as renderComponent, r as renderTemplate, m as maybeRenderHead, l as renderScript } from '../chunks/astro/server_bfQdwCeF.mjs';
import { $ as $$Layout } from '../chunks/Layout_B-tuoUaj.mjs';
/* empty css                                      */
export { renderers } from '../renderers.mjs';

const $$Astro = createAstro();
const $$Rianalysis = createComponent(async ($$result, $$props, $$slots) => {
  const Astro2 = $$result.createAstro($$Astro, $$props, $$slots);
  Astro2.self = $$Rianalysis;
  const { dashboardId } = Astro2.props;
  return renderTemplate`${renderComponent($$result, "Layout", $$Layout, { "title": "Analyse", "data-astro-cid-sygai7lg": true }, { "default": async ($$result2) => renderTemplate` ${maybeRenderHead()}<div class="p-8" data-astro-cid-sygai7lg> <h1 class="text-3xl font-bold mb-6" data-astro-cid-sygai7lg>Analyse</h1> <div id="dashboard-container" data-astro-cid-sygai7lg></div> <!-- Client-side script --> ${renderScript($$result2, "/home/sla/projects/EMS/frontend/src/pages/rianalysis.astro?astro&type=script&index=0&lang.ts")}  </div> ` })}`;
}, "/home/sla/projects/EMS/frontend/src/pages/rianalysis.astro", void 0);

const $$file = "/home/sla/projects/EMS/frontend/src/pages/rianalysis.astro";
const $$url = "/rianalysis";

const _page = /*#__PURE__*/Object.freeze(/*#__PURE__*/Object.defineProperty({
	__proto__: null,
	default: $$Rianalysis,
	file: $$file,
	url: $$url
}, Symbol.toStringTag, { value: 'Module' }));

const page = () => _page;

export { page };
