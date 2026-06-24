Feature: Analyse (Analysis) module
  As a user
  I want deep consumption, benchmark and efficiency analyses
  So that I can find savings and verify district-heating performance

  Background:
    Given I am in the company context "Bolag AB Janus"

  Scenario: Bygningsbenchmark shows a benchmark gauge and a detail grid
    When I open Analyse > Bygningsbenchmark
    Then the URL is "/App/company/997/analysis/insights"
    And I see a performance gauge with deviation and savings metrics for a chosen period and area basis
    And I see a groupable, searchable building detail grid
    And I can filter by period, energy form, key basis, reference-building country and building filters

  Scenario: Forbrug is the consumption analysis with budget comparison
    When I open Analyse > Forbrug
    Then the URL is "/App/company/997/analysis/consumption/"
    And I see a filter panel, a meter list and a per-meter graph area
    And I can choose period length, start/end month, meter scope, tags and meter types
    And I can compare against budget or the same period last year
    And I can apply degree-and-date correction and accumulation
    And a long data fetch shows a progress/please-wait state

  Scenario: Standbyanalyse compares standby vs operating consumption
    When I open Analyse > Standbyanalyse
    Then the URL is "/App/company/997/analysis/standby"
    And I see energy-type tabs for EL, Varme and Vand
    And I see a per-building table with standby, operating, totals, per-hour rates and standby shares
    And I can switch the measure between cost, energy and CO₂e

  Scenario Outline: Delta-T efficiency pages render one panel per meter
    When I open Analyse > <page>
    Then the URL is "/App/company/997/<slug>"
    And I see one panel per meter, each with a Highcharts chart and a KPI table
    And each panel has "Nøgletal" and "Metadata" tabs
    And the meters are sourced from the cooling query endpoint

    Examples:
      | page        | slug                          |
      | Afkøling    | analysis/consumption_cooling  |
      | Fjernkøling | analysis/consumption_heating  |

  Scenario: Tjek is the phasing-out traffic-light check
    When I open Analyse > Tjek
    Then the URL is "/App/company/997/check"
    And I see a consumption-vs-budget deviation matrix with year, month and period selectors
    And it is marked as being phased out

  Scenario: Gated analysis pages are hidden when not licensed
    Then "Raven Residential" and "Netværksdiagnostik" are not shown when the feature is not enabled for the context
