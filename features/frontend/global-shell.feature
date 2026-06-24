Feature: Global application shell
  As a user of our EMS frontend
  I want a persistent shell with context selection, header tools and module navigation
  So that every page is scoped, reachable and consistent

  Background:
    Given I am authenticated
    And the shell has loaded the startup, rights and navigation-tree data

  Scenario: Shell layout regions are present
    Then I see a collapsible left hierarchy/context panel
    And I see a header with a breadcrumb and a right-aligned icon toolbar
    And I see a horizontal module bar below the header
    And I see the routed page content area
    And the layout uses a CSS grid (not flexbox)

  Scenario: Selecting a context scopes the whole app
    Given the hierarchy panel shows the tabs "Hierarki", "Fælles grp." and "Privat grp."
    And it shows a department picker and a company picker
    When I select the company "Bolag AB Janus"
    Then the URL changes to "/App/company/997/overview/dashboard"
    And the breadcrumb reflects the selected company, module and page
    And subsequent page data requests carry the selected contextType and contextId

  Scenario: Hierarchy tree reflects the context type
    When I expand the hierarchy panel
    Then I see a tree of department -> company -> building -> area nodes
    And selecting a node re-scopes the app to that node

  Scenario: Header toolbar exposes the global tools
    Then the header toolbar offers notifications, news, favourites, search, an app launcher, help and a user menu

  Scenario: Module bar opens a flyout on activation
    When I activate the "Resource Insights" module in the module bar
    Then a flyout reveals its pages "Overblik", "Analyse" and "Energimodel"
    And activating a page navigates to that page within the current context

  Scenario Outline: Top-level modules are present and ordered
    Then the module bar contains the module "<module>"

    Examples:
      | module            |
      | Oversigt          |
      | Resource Insights |
      | Call to action    |
      | Analyse           |
      | Overvågning       |
      | Målere            |
      | Klimaregnskab     |
      | Aktiv Styring     |
      | Rapportering      |
      | Opsætning         |

  Scenario: Menu visibility follows the user's rights
    Given the user rights response omits a feature
    Then the corresponding module or page is not shown in the module bar
