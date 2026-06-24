Feature: Målere (Meters) module
  As a user
  I want a filterable registry of all meters in the context
  So that I can find and inspect meters

  Background:
    Given I am in the company context "Bolag AB Janus"

  Scenario: Meter registry grid
    When I open the Målere module
    Then the URL is "/App/company/997/meters"
    And I see a header "Målere for Bolag AB Janus" with the meter count and selected count
    And I see a results grid with columns building, meter type, hierarchy, meter designation, unique id and tags
    And the meter-type cell shows an energy-type icon and label
    And the grid data is sourced from the meters list endpoint

  Scenario: Searching and selecting meters
    Then I can quick-search by unique id or designation
    And I can select rows via a header checkbox or per row
    And the grid is paginated with a configurable page size

  Scenario: Filtering meters
    When I open the filter overlay
    Then I can filter by building use and buildings
    And I can filter by meter types with an AND/OR combinator, tags, meter details and behaviour
    And applying the filters re-queries the meters list with the chosen filter body

  Scenario: Create meter entry point
    Then I see a "Opret måler" action that opens a server-rendered create form (not exercised here)

  Scenario: Module is a childless top-level
    Then activating "Målere" navigates straight to the registry without a flyout
    And the second "Målere" entry badged "Nyt" is treated as the same route family
