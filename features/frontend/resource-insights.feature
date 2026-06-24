Feature: Resource Insights module
  As a user
  I want filter-driven analytics over my buildings and meters
  So that I can explore totals, time series and energy models

  Background:
    Given I am in the company context "Bolag AB Janus"

  Scenario: Shared filter bar across Resource Insights pages
    When I open any Resource Insights page
    Then I see a filter bar with a date-range picker, building/tag/energy-class multi-selects,
      a resolution toggle, an energy/KPI measure toggle, an export action and a reset action
    And I can save the current filter as a named custom filter

  Scenario: Overblik shows totals and a meter grid
    When I open Resource Insights > Overblik
    Then the URL is "/App/company/997/resource_insights/resource_insights_overview"
    And I see a totals panel with one figure per resource/unit present (kr., t CO₂, GWh, m³, kWh, km, ...)
    And I see a selectable, paginated meter grid
    And applying a filter re-queries the consumption-analysis overview and KPI endpoints

  Scenario: Totals panel is unit-driven, not hard-coded
    Given the selected buildings expose a given set of resources
    Then the totals panel renders exactly one figure per exposed unit

  Scenario: Analyse shows a consumption time-series chart
    When I open Resource Insights > Analyse
    Then the URL is "/App/company/997/resource_insights/resource_insights_analysis"
    And I see a Highcharts time-series chart with per-resource totals
    And the resolution toggle offers year, month, week, day, hour, 30 min and 15 min
    And changing filters re-queries the consumption-analysis details and KPI endpoints

  Scenario: Energimodel lists energy models per building
    When I open Resource Insights > Energimodel
    Then the URL is "/App/company/997/energy_model"
    And I see a selectable, searchable grid of energy models with a selected-count header
    And I can filter by building use, city, country and quality
    And the grid is sourced from the energy-models buildings-overview endpoint
