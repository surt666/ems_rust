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

  Scenario: Per-row action menu
    When I open the "..." action menu on a meter row
    Then I see the items "Vis flere detaljer", "Redigér måler", "Rediger målertags", "Gå til forbrug", "Gå til datatilegnelse", "Deaktivér måler" and "Slet måler"
    And "Deaktivér måler" and "Slet måler" are marked as destructive
    And "Gå til datatilegnelse" navigates to the data-acquisition page for that meter

  Scenario: Datatilegnelse (data acquisition) page
    When I choose "Gå til datatilegnelse" from a meter's action menu
    Then the URL is "/App/company/997/datasource?maalerId=<id>"
    And I see the meter name, EMS id and physical meter number with a status toggle "I drift" / "Under installation"
    And I see a datasource block with a logger type from the source vocabulary (Electrocom, Danfoss, Techem, Datahub, Manual, CsvFile, ...) and a validity date range
    And I see a counter-registers grid with columns unit, multiplier, conversion factor, start value, DAQ ID and "reported in consumption"
    And I see a readings grid per register with columns date, estimated counter reading, reading and consumption
    And the readings grid data is sourced from the measurements endpoint

  Scenario: Per-reading correction menu
    When I open the "..." menu on a reading row
    Then I see the items "Indsæt aflæsning over", "Indsæt aflæsning under", "Rediger aflæsning", "Slet aflæsning", "Justér anslået tællerstand", "Gå til forbrug", "Tællervending", "Målerskifte", "Split datakilde her" and "Vis ændringslog"
    And these expose the start-value, meter-change and counter-rollover operations referenced by the physical-meter-reading base-value plan
