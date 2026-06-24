Feature: Klimaregnskab (Climate / carbon accounting) module
  As a user
  I want GHG and CSRD reporting and emission-factor browsing
  So that I can report and understand my emissions

  Background:
    Given I am in the company context "Bolag AB Janus"

  Scenario: GHG report layout
    When I open Klimaregnskab > Klimaregnskab
    Then the URL is "/App/company/997/environmental_accounting_root/environmental_accounting_new"
    And I see a scope donut chart with a total-emission centre and a Scope 1/2/3 legend
    And I see a per-category stacked bar chart with a Scope 1/2/3 switch
    And I see three scope cards each with a value and a percentage gauge
    And I see an emission-statement section below the cards

  Scenario: GHG report parameters are server-side
    Then I can choose location-based vs market-based emissions
    And I can choose a reporting period and building filters
    When I change the location basis or the scope switch
    Then the report re-queries the emission-statement and chart endpoints with those parameters

  Scenario: CSRD report is a groupable grid
    When I open Klimaregnskab > CSRD-rapport
    Then the URL is "/App/company/997/environmental_accounting_root/csrd"
    And I see a date-range picker and an export action
    And I see a groupable, searchable CSRD line-item grid
    And the grid is sourced from the CSRD overview endpoint

  Scenario: Emissionsfaktorer is an expandable navigator
    When I open Klimaregnskab > Emissionsfaktorer
    Then the URL is "/App/company/997/environmental_accounting_root/emission_factors"
    And I see a navigator listing emission types (heat, water, electricity, transport distance, resources, transport fuel)
    And expanding an emission type drills into a region hierarchy to reach the factors
    And the hierarchy is sourced from the emission-types hierarchy endpoint

  Scenario: Phasing-out duplicate is deprioritised
    Then the second "Klimaregnskab" entry marked "Udfases" maps to the same report surface and is low priority to recreate
