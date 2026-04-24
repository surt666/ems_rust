import { e as createComponent, m as maybeRenderHead, l as renderScript, r as renderTemplate, k as renderComponent } from '../chunks/astro/server_bfQdwCeF.mjs';
import { $ as $$Layout } from '../chunks/Layout_B-tuoUaj.mjs';
import 'clsx';
/* empty css                                 */
export { renderers } from '../renderers.mjs';

const $$Login = createComponent(async ($$result, $$props, $$slots) => {
  return renderTemplate`${maybeRenderHead()}<div class="login-container" data-astro-cid-b2fdlob7> <div class="login-form" data-astro-cid-b2fdlob7> <h1 class="text-2xl font-bold text-center mb-6 text-gray-800" data-astro-cid-b2fdlob7>Welcome to EMS</h1> <form id="loginForm" class="space-y-4" data-astro-cid-b2fdlob7> <div data-astro-cid-b2fdlob7> <label for="username" class="block text-sm font-medium text-gray-700 mb-1" data-astro-cid-b2fdlob7>Username</label> <input type="text" id="username" name="username" required class="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500" placeholder="Enter your username" data-astro-cid-b2fdlob7> </div> <div data-astro-cid-b2fdlob7> <label for="password" class="block text-sm font-medium text-gray-700 mb-1" data-astro-cid-b2fdlob7>Password</label> <input type="password" id="password" name="password" required class="w-full px-3 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500" placeholder="Enter your password" data-astro-cid-b2fdlob7> </div> <button type="submit" id="loginButton" class="w-full bg-blue-600 text-white py-2 px-4 rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 transition duration-200" data-astro-cid-b2fdlob7>
Sign In
</button> </form> <div id="errorMessage" class="hidden mt-4 p-3 bg-red-100 border border-red-400 text-red-700 rounded" data-astro-cid-b2fdlob7></div> <div id="loadingMessage" class="hidden mt-4 p-3 bg-blue-100 border border-blue-400 text-blue-700 rounded text-center" data-astro-cid-b2fdlob7>
Signing in...
</div> </div> </div> ${renderScript($$result, "/home/sla/projects/EMS/frontend/src/components/Login.astro?astro&type=script&index=0&lang.ts")} `;
}, "/home/sla/projects/EMS/frontend/src/components/Login.astro", void 0);

const $$Index = createComponent(($$result, $$props, $$slots) => {
  return renderTemplate`${renderComponent($$result, "Layout", $$Layout, { "title": "ESM Light" }, { "default": ($$result2) => renderTemplate` ${maybeRenderHead()}<div x-show="!isAuthenticated"> ${renderComponent($$result2, "Login", $$Login, {})} </div> <div x-show="isAuthenticated"> <main class="p-4 text-white text-xl"> <div class="mb-6"> <h1 class="text-6xl mb-2 text-center font-bold">Welcome to <span class="text-gradient">ESM</span></h1> </div> <div id="node-info"></div> <!--iframe width="1260" height="550" src="https://eu-central-1.quicksight.aws.amazon.com/sn/embed/share/accounts/891377204778/dashboards/a25fd080-b64d-4dbe-9c3e-ccf044696484/sheets/a25fd080-b64d-4dbe-9c3e-ccf044696484_fd7d45ed-974f-46fa-b882-f597733181c0/visuals/a25fd080-b64d-4dbe-9c3e-ccf044696484_ea756699-5e54-4289-8f36-5a453a0f7b3b?directory_alias=QuickDev"></iframe>
			<iframe width="1260" height="550" src="https://eu-central-1.quicksight.aws.amazon.com/sn/embed/share/accounts/891377204778/dashboards/7c52c7f5-be48-42cb-bc9a-0f95b635be80/sheets/7c52c7f5-be48-42cb-bc9a-0f95b635be80_be3bacd3-18fa-49fd-8bcd-80c10770a7ef/visuals/7c52c7f5-be48-42cb-bc9a-0f95b635be80_430b7937-29a8-4954-8428-2a0097b475e8?directory_alias=QuickDev&daqid=daq:mivo_json_v1:000406:23495955:watervolume&year=2025&month=4&day=0"></iframe>
			<iframe
				width="1260"
				height="1320"
				src="https://eu-central-1.quicksight.aws.amazon.com/sn/embed/share/accounts/891377204778/dashboards/752919ae-c026-44a6-a786-9675cfe0e9bf?directory_alias=QuickDev">
			</iframe --> </main> </div> ` })} ${renderScript($$result, "/home/sla/projects/EMS/frontend/src/pages/index.astro?astro&type=script&index=0&lang.ts")}`;
}, "/home/sla/projects/EMS/frontend/src/pages/index.astro", void 0);

const $$file = "/home/sla/projects/EMS/frontend/src/pages/index.astro";
const $$url = "";

const _page = /*#__PURE__*/Object.freeze(/*#__PURE__*/Object.defineProperty({
	__proto__: null,
	default: $$Index,
	file: $$file,
	url: $$url
}, Symbol.toStringTag, { value: 'Module' }));

const page = () => _page;

export { page };
