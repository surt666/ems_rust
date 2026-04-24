import { e as createComponent, k as renderComponent, r as renderTemplate, m as maybeRenderHead } from '../chunks/astro/server_bfQdwCeF.mjs';
import { $ as $$Layout } from '../chunks/Layout_B-tuoUaj.mjs';
/* empty css                                    */
export { renderers } from '../renderers.mjs';

const $$Userlist = createComponent(async ($$result, $$props, $$slots) => {
  return renderTemplate`${renderComponent($$result, "Layout", $$Layout, { "title": "Brugerliste for: Enity", "data-astro-cid-p6j65onp": true }, { "default": async ($$result2) => renderTemplate` ${maybeRenderHead()}<div class="min-h-screen bg-gray-50" data-astro-cid-p6j65onp> <div class="max-w-7xl mx-auto" data-astro-cid-p6j65onp> <div class="grid grid-cols-[1fr_auto_auto] gap-6 px-6 py-3 text-sm" x-data="{
				deleteDialogOpen: false,
				emailToDelete: null,
				openDeleteDialog(email) {
					this.emailToDelete = email;
					this.deleteDialogOpen = true;
					this.$refs.deleteDialog.showModal();
				},
				closeDeleteDialog() {
					this.deleteDialogOpen = false;
					this.$refs.deleteDialog.close();
					this.emailToDelete = null;
				},
				async deleteUser() {
					if (!this.emailToDelete) return;

					try {
						const response = await fetch('https://externalapi.dev-ems.enity.io/hierarchy/command', {
							method: 'POST',
							headers: {
								'Content-Type': 'application/json',
							},
							body: JSON.stringify({
								action: 'delete_user',
								data: {
									email: this.emailToDelete
								}
							})
						});

						if (!response.ok) {
							const errorText = await response.text();
							alert('Fejl ved sletning: ' + errorText);
							return;
						}

						this.closeDeleteDialog();
						location.reload();
					} catch (error) {
						alert('Fejl ved sletning: ' + error.message);
					}
				}
			}" @delete-user.window="openDeleteDialog($event.detail.email)" data-astro-cid-p6j65onp> <div data-astro-cid-p6j65onp></div> <a href="#" _="on click halt the event then call #create-user-dialog.showModal()" class="text-blue-600 hover:text-blue-800" data-astro-cid-p6j65onp>Opret bruger</a> <a href="#" class="text-blue-600 hover:text-blue-800" data-astro-cid-p6j65onp>Genindlæs</a> <!-- Create User Dialog --> <dialog id="create-user-dialog" _="on click if event.target == me then call me.close()" class="rounded-lg shadow-2xl p-0 w-[800px] m-auto" data-astro-cid-p6j65onp> <div class="bg-gray-200 bg-opacity-95 px-6 py-4 grid grid-cols-[1fr_auto] items-center border-b border-gray-400" data-astro-cid-p6j65onp> <h2 class="text-2xl font-bold text-gray-800" data-astro-cid-p6j65onp>OPRET BRUGER - ENITY</h2> <button _="on click call #create-user-dialog.close()" class="text-3xl text-gray-600 hover:text-gray-800 leading-none" data-astro-cid-p6j65onp>×</button> </div> <div class="p-6 bg-white bg-opacity-95" data-astro-cid-p6j65onp> <!-- Tab Navigation --> <div class="grid grid-cols-[auto_auto_auto] gap-2 mb-6 border-b border-gray-300" data-astro-cid-p6j65onp> <button id="tab-btn-bruger" class="px-6 py-3 bg-gray-800 text-white font-semibold" _="on click
									add @style='display:none' to #tab-dataadgang
									remove @style from #tab-bruger-detaljer
									add .bg-gray-800 to me
									add .text-white to me
									remove .bg-gray-300 from me
									remove .text-gray-600 from me
									add .bg-gray-300 to #tab-btn-dataadgang
									add .text-gray-600 to #tab-btn-dataadgang
									remove .bg-gray-800 from #tab-btn-dataadgang
									remove .text-white from #tab-btn-dataadgang" data-astro-cid-p6j65onp>
Bruger detaljer
</button> <button id="tab-btn-dataadgang" class="px-6 py-3 bg-gray-300 text-gray-600 font-semibold hover:bg-gray-400" _="on click
									add @style='display:none' to #tab-bruger-detaljer
									remove @style from #tab-dataadgang
									add .bg-gray-800 to me
									add .text-white to me
									remove .bg-gray-300 from me
									remove .text-gray-600 from me
									add .bg-gray-300 to #tab-btn-bruger
									add .text-gray-600 to #tab-btn-bruger
									remove .bg-gray-800 from #tab-btn-bruger
									remove .text-white from #tab-btn-bruger
									then send dataadgangShown to body" data-astro-cid-p6j65onp>
Dataadgang
</button> </div> <!-- Tab Content --> <div id="form-error" class="hidden text-red-600 font-bold mb-4" data-astro-cid-p6j65onp></div> <!-- Tab 1: Bruger detaljer --> <div id="tab-bruger-detaljer" data-astro-cid-p6j65onp> <form id="create-user-form" class="space-y-4" hx-post="https://externalapi.dev-ems.enity.io/hierarchy/command" hx-swap="none" hx-on::after-request="if(event.detail.elt.id === 'create-user-form' && event.detail.successful) { document.querySelector('dialog').close(); location.reload(); } else if(event.detail.elt.id === 'create-user-form') { document.getElementById('form-error').textContent = event.detail.xhr.responseText; document.getElementById('form-error').classList.remove('hidden'); }" data-astro-cid-p6j65onp> <input type="hidden" name="action" value="create_user" data-astro-cid-p6j65onp> <div class="grid grid-cols-[200px_1fr_auto] gap-4 items-center" data-astro-cid-p6j65onp> <label class="font-bold text-gray-800" data-astro-cid-p6j65onp>Fuldt navn</label> <input type="text" name="data.name" required class="border border-gray-300 rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" data-astro-cid-p6j65onp> <span class="text-red-500 font-bold" data-astro-cid-p6j65onp>*</span> </div> <div class="grid grid-cols-[200px_1fr_auto_auto_auto] gap-4 items-center" data-astro-cid-p6j65onp> <label class="font-bold text-gray-800" data-astro-cid-p6j65onp>Email</label> <input type="email" name="data.email" required class="border border-gray-300 rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" data-astro-cid-p6j65onp> <span class="text-red-500 font-bold" data-astro-cid-p6j65onp>*</span> <button type="button" class="bg-red-600 text-white px-3 py-2 rounded hover:bg-red-700" data-astro-cid-p6j65onp> <span class="font-bold" data-astro-cid-p6j65onp>•••</span> </button> <span data-astro-cid-p6j65onp></span> </div> <div class="grid grid-cols-[200px_1fr] gap-4 items-center" data-astro-cid-p6j65onp> <label class="font-bold text-gray-800" data-astro-cid-p6j65onp>Brugerprofil</label> <select name="data.profile" class="border border-gray-300 rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" hx-get="https://externalapi.dev-ems.enity.io/hierarchy/query/profiles" hx-request="{&quot;noHeaders&quot;: true}" hx-trigger="intersect once" hx-params="none" hx-swap="innerHTML" data-astro-cid-p6j65onp> <option value="Developer" data-astro-cid-p6j65onp>Udvikler</option> <option value="Standard" selected data-astro-cid-p6j65onp>Standardbruger</option> <option value="Technician" data-astro-cid-p6j65onp>Tekniker</option> <option value="Reader" data-astro-cid-p6j65onp>Læser</option> <option value="SysAdm" data-astro-cid-p6j65onp>System Administrator</option> </select> </div> <div class="grid grid-cols-[200px_1fr] gap-4 items-center" data-astro-cid-p6j65onp> <label class="font-bold text-gray-800" data-astro-cid-p6j65onp>Sprog</label> <select name="data.language" class="border border-gray-300 rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" hx-get="https://externalapi.dev-ems.enity.io/hierarchy/query/languages" hx-request="{&quot;noHeaders&quot;: true}" hx-trigger="intersect once" hx-params="none" hx-swap="innerHTML" data-astro-cid-p6j65onp> <option value="Danish" selected data-astro-cid-p6j65onp>Dansk (Dansk)</option> <option value="English" data-astro-cid-p6j65onp>English (English)</option> </select> </div> <div class="grid grid-cols-[200px_1fr] gap-4 items-center" data-astro-cid-p6j65onp> <label class="font-bold text-gray-800" data-astro-cid-p6j65onp>Valuta</label> <select name="data.currency" class="border border-gray-300 rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" hx-get="https://externalapi.dev-ems.enity.io/hierarchy/query/currencies" hx-request="{&quot;noHeaders&quot;: true}" hx-trigger="intersect once" hx-params="none" hx-swap="innerHTML" data-astro-cid-p6j65onp> <option value="DKK" selected data-astro-cid-p6j65onp>DKK - danske kroner</option> <option value="EUR" data-astro-cid-p6j65onp>EUR - Euro</option> </select> </div> <div class="grid grid-cols-[200px_1fr] gap-4 items-center" data-astro-cid-p6j65onp> <label class="font-bold text-gray-800" data-astro-cid-p6j65onp>Node ID (for testing)</label> <input type="text" name="data.node_id" value="1" class="border border-gray-300 rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" data-astro-cid-p6j65onp> </div> <div class="grid grid-cols-[200px_1fr] gap-4 items-center" data-astro-cid-p6j65onp> <label class="font-bold text-gray-800" data-astro-cid-p6j65onp>Permission</label> <select name="data.permissions" class="border border-gray-300 rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" hx-get="https://externalapi.dev-ems.enity.io/hierarchy/query/permissions" hx-request="{&quot;noHeaders&quot;: true}" hx-trigger="intersect once" hx-params="none" hx-swap="innerHTML" data-astro-cid-p6j65onp> <option value="READ" selected data-astro-cid-p6j65onp>Read</option> <option value="WRITE" data-astro-cid-p6j65onp>Write</option> </select> </div> <div class="grid grid-cols-[200px_1fr] gap-4 items-center" data-astro-cid-p6j65onp> <label class="font-bold text-gray-800" data-astro-cid-p6j65onp>Node Type</label> <select name="data.nodetypes" class="border border-gray-300 rounded px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" hx-get="https://externalapi.dev-ems.enity.io/hierarchy/query/nodetypes" hx-request="{&quot;noHeaders&quot;: true}" hx-trigger="intersect once" hx-params="none" hx-swap="innerHTML" data-astro-cid-p6j65onp> <option value="Partner" selected data-astro-cid-p6j65onp>Partner</option> <option value="Company" data-astro-cid-p6j65onp>Company</option> </select> </div> </form> </div> <!-- Tab 2: Dataadgang --> <div id="tab-dataadgang" style="display:none;" data-astro-cid-p6j65onp> <!-- Search field --> <div class="mb-4" data-astro-cid-p6j65onp> <input type="text" placeholder="Søg efter afdeling, firma, ejendomme, gruppering eller bygninger" class="w-full border border-gray-300 rounded px-4 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500" data-astro-cid-p6j65onp> </div> <!-- Hierarchy table --> <div class="border border-gray-200 rounded grid grid-cols-[auto_auto_auto_1fr_auto_auto_auto] gap-x-4 gap-y-0" id="permission-grid" data-astro-cid-p6j65onp> <!-- Table header --> <div class="col-span-7 grid grid-cols-subgrid bg-gray-100 p-3 border-b border-gray-200 items-center font-bold text-sm" data-astro-cid-p6j65onp> <span class="w-5 h-5" data-astro-cid-p6j65onp></span> <span class="w-5 h-5" data-astro-cid-p6j65onp></span> <input type="checkbox" class="w-5 h-5" data-astro-cid-p6j65onp> <span data-astro-cid-p6j65onp>Navn</span> <div class="grid grid-cols-[auto_1fr] gap-2 items-center" data-astro-cid-p6j65onp> <svg class="w-5 h-5 text-blue-500" fill="none" stroke="currentColor" viewBox="0 0 24 24" data-astro-cid-p6j65onp><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15.232 5.232l3.536 3.536m-2.036-5.036a2.5 2.5 0 113.536 3.536L6.5 21.036H3v-3.572L16.732 3.732z" data-astro-cid-p6j65onp></path></svg> <span data-astro-cid-p6j65onp>Må indtaste<br data-astro-cid-p6j65onp>aflæsninger</span> </div> <div class="grid grid-cols-[auto_1fr] gap-2 items-center" data-astro-cid-p6j65onp> <svg class="w-5 h-5 text-green-500" fill="none" stroke="currentColor" viewBox="0 0 24 24" data-astro-cid-p6j65onp><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 5a2 2 0 012-2h3.28a1 1 0 01.948.684l1.498 4.493a1 1 0 01-.502 1.21l-2.257 1.13a11.042 11.042 0 005.516 5.516l1.13-2.257a1 1 0 011.21-.502l4.493 1.498a1 1 0 01.684.949V19a2 2 0 01-2 2h-1C9.716 21 3 14.284 3 6V5z" data-astro-cid-p6j65onp></path></svg> <span data-astro-cid-p6j65onp>Primær kontakt</span> </div> <div class="grid grid-cols-[auto_1fr] gap-2 items-center" data-astro-cid-p6j65onp> <svg class="w-5 h-5 text-orange-500" fill="none" stroke="currentColor" viewBox="0 0 24 24" data-astro-cid-p6j65onp><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" data-astro-cid-p6j65onp></path></svg> <span data-astro-cid-p6j65onp>Øvrige<br data-astro-cid-p6j65onp>kontaktpersoner</span> </div> </div> <!-- Dynamic hierarchy nodes loaded via HTMX --> <div hx-get="https://externalapi.dev-ems.enity.io/hierarchy/query/nodes?permissions=true" hx-vals="js:{user: localStorage.getItem('loginId')}" hx-request="{&quot;noHeaders&quot;: true}" hx-trigger="dataadgangShown from:body once" hx-swap="beforeend" hx-target="#permission-grid" data-astro-cid-p6j65onp></div> </div> </div> </div> <!-- Dialog Footer --> <div class="bg-gray-100 bg-opacity-95 px-6 py-4 grid grid-cols-[auto_auto_1fr_auto_auto] gap-2 border-t border-gray-300" data-astro-cid-p6j65onp> <button type="submit" form="create-user-form" class="bg-orange-500 text-white px-6 py-2 rounded font-bold hover:bg-orange-600" data-astro-cid-p6j65onp>Gem</button> <button _="on click call #create-user-dialog.close()" class="bg-orange-500 text-white px-6 py-2 rounded font-bold hover:bg-orange-600" data-astro-cid-p6j65onp>Luk</button> <div data-astro-cid-p6j65onp></div> <button _="on click send click to #tab-btn-bruger" class="bg-gray-300 text-gray-700 px-6 py-2 rounded font-bold hover:bg-gray-400" data-astro-cid-p6j65onp>Forrige</button> <button _="on click send click to #tab-btn-dataadgang" class="bg-orange-500 text-white px-6 py-2 rounded font-bold hover:bg-orange-600" data-astro-cid-p6j65onp>Næste</button> </div> </dialog> <!-- Delete User Confirmation Dialog --> <dialog x-ref="deleteDialog" @click.self="closeDeleteDialog()" class="rounded-lg shadow-2xl p-0 w-[500px] m-auto" data-astro-cid-p6j65onp> <div class="bg-red-100 bg-opacity-95 px-6 py-4 grid grid-cols-[1fr_auto] items-center border-b border-red-300" data-astro-cid-p6j65onp> <h2 class="text-2xl font-bold text-red-800" data-astro-cid-p6j65onp>SLET BRUGER</h2> <button @click="closeDeleteDialog()" class="text-3xl text-red-600 hover:text-red-800 leading-none" data-astro-cid-p6j65onp>×</button> </div> <div class="p-6 bg-white bg-opacity-95" data-astro-cid-p6j65onp> <p class="text-gray-800 mb-4" data-astro-cid-p6j65onp>Er du sikker på, at du vil slette denne bruger?</p> <p class="text-sm text-gray-600 mb-4" data-astro-cid-p6j65onp>
Bruger ID: <span class="font-mono font-bold" x-text="emailToDelete" data-astro-cid-p6j65onp></span> </p> <p class="text-sm text-red-600 font-bold" data-astro-cid-p6j65onp>
⚠️ Dette kan ikke fortrydes!
</p> </div> <!-- Dialog Footer --> <div class="bg-gray-100 bg-opacity-95 px-6 py-4 grid grid-cols-[auto_1fr_auto] gap-2 border-t border-gray-300" data-astro-cid-p6j65onp> <button @click="closeDeleteDialog()" class="px-4 py-2 bg-gray-300 text-gray-800 rounded hover:bg-gray-400" data-astro-cid-p6j65onp>
Annuller
</button> <div data-astro-cid-p6j65onp></div> <button @click="deleteUser()" class="px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700" data-astro-cid-p6j65onp>
Slet
</button> </div> </dialog> </div> <div class="px-6 py-2" data-astro-cid-p6j65onp> <h1 class="text-base font-normal text-gray-700" data-astro-cid-p6j65onp>Brugerliste for: Enity</h1> </div> <div class="mx-6 mt-4 bg-white border border-gray-300 rounded-sm" data-astro-cid-p6j65onp> <div class="bg-gray-200 px-4 py-3 font-bold text-sm border-b border-gray-300" data-astro-cid-p6j65onp>
▼ ALLE BRUGERE MED DATAADGANG
</div> <div class="overflow-x-auto" data-astro-cid-p6j65onp> <table class="w-full text-sm" data-astro-cid-p6j65onp> <thead data-astro-cid-p6j65onp> <tr class="bg-gray-100 border-b border-gray-300" data-astro-cid-p6j65onp> <th class="px-4 py-3 text-left font-bold text-xs text-gray-700" data-astro-cid-p6j65onp>NAVN</th> <th class="px-4 py-3 text-left font-bold text-xs text-gray-700" data-astro-cid-p6j65onp>EMAIL</th> <th class="px-4 py-3 text-left font-bold text-xs text-gray-700" data-astro-cid-p6j65onp>BRUGERPROFIL</th> <th class="px-4 py-3 text-left font-bold text-xs text-gray-700" data-astro-cid-p6j65onp>SPROG</th> <th class="px-4 py-3 text-left font-bold text-xs text-gray-700" data-astro-cid-p6j65onp>VALUTA</th> <th class="px-4 py-3 text-left font-bold text-xs text-gray-700" data-astro-cid-p6j65onp>HANDLINGER</th> </tr> </thead> <tbody hx-get="https://externalapi.dev-ems.enity.io/hierarchy/query/users" hx-request="{&quot;noHeaders&quot;: true}" hx-trigger="load" hx-swap="innerHTML" data-astro-cid-p6j65onp> <tr data-astro-cid-p6j65onp> <td class="px-4 py-3 text-center text-gray-500" colspan="5" data-astro-cid-p6j65onp>Loading users...</td> </tr> </tbody> </table> </div> </div> </div> </div> ` })} `;
}, "/home/sla/projects/EMS/frontend/src/pages/userlist.astro", void 0);

const $$file = "/home/sla/projects/EMS/frontend/src/pages/userlist.astro";
const $$url = "/userlist";

const _page = /*#__PURE__*/Object.freeze(/*#__PURE__*/Object.defineProperty({
	__proto__: null,
	default: $$Userlist,
	file: $$file,
	url: $$url
}, Symbol.toStringTag, { value: 'Module' }));

const page = () => _page;

export { page };
