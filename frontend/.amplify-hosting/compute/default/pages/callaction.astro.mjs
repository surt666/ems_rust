import { e as createComponent, k as renderComponent, r as renderTemplate, m as maybeRenderHead } from '../chunks/astro/server_bfQdwCeF.mjs';
import { $ as $$Layout } from '../chunks/Layout_B-tuoUaj.mjs';
export { renderers } from '../renderers.mjs';

const $$Callaction = createComponent(($$result, $$props, $$slots) => {
  return renderTemplate`${renderComponent($$result, "Layout", $$Layout, { "title": "Call to Action" }, { "default": ($$result2) => renderTemplate` ${maybeRenderHead()}<div class="p-8"> <h1 class="text-3xl font-bold mb-6">Call to Action</h1> <p class="text-gray-600">Handlingsanbefalinger og energioptimering.</p> </div> ` })}`;
}, "/home/sla/projects/EMS/frontend/src/pages/callaction.astro", void 0);

const $$file = "/home/sla/projects/EMS/frontend/src/pages/callaction.astro";
const $$url = "/callaction";

const _page = /*#__PURE__*/Object.freeze(/*#__PURE__*/Object.defineProperty({
	__proto__: null,
	default: $$Callaction,
	file: $$file,
	url: $$url
}, Symbol.toStringTag, { value: 'Module' }));

const page = () => _page;

export { page };
