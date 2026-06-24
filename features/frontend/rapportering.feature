Feature: Aktiv Styring, Rapportering & Call to action
  As a user
  I want reporting tools, data export and a savings call-to-action
  So that I can share reports and act on the biggest opportunities

  Background:
    Given I am in the company context "Bolag AB Janus"

  Scenario: Aktiv Styring is an upsell landing
    When I open the Aktiv Styring module
    Then the URL is "/App/company/997/active_management"
    And I see a promotional landing page with no operational components
    And it is gated behind a feature flag in our recreation

  Scenario: Custom reports registry and builder
    When I open Rapportering > Brugertilpassede rapporter
    Then the URL is "/App/company/997/reports/custom"
    And I see sections for reports I created and reports others created
    And empty sections show a "no reports found" empty state
    And I see a "Opret ny rapport" action that opens the report builder (not exercised here)

  Scenario: Building report subscriptions
    When I open Rapportering > Bygningsrapporten
    Then the URL is "/App/company/997/reports/buildings"
    And I see a subscriptions grid grouped by building with subscriber, grouping, contact, profile, schedule, annual cost and subscribed columns
    And I can filter by users and buildings
    And I can switch between Alle, Frameldt and Tilmeldt
    And annual-cost figures come from the meter-data group-by endpoint

  Scenario: Consumption-data export job
    When I open Rapportering > Eksporter forbrugsdata
    Then the URL is "/App/company/997/consumption_export"
    And I see a filter form with period, resolution, building/meter/tag multi-selects and options like include sub-meters and climate correction
    And I see a live preview of building, meter and data-line counts plus an estimated export time
    And I see an export-files table that lists generated files with date, counts and status
    When I queue an export
    Then the job runs in the background and the files table polls the job-status endpoint

  Scenario: Call to action lists the biggest waste opportunities
    When I open the Call to action module
    Then the URL is "/App/company/997/call_to_action"
    And I see totals for estimated extra cost per year, extra cost to date and unacknowledged alarms
    And I see a groupable grid keyed by building/meter/alarm with extra-cost and latest-alarm columns
    And it is sourced from the call-to-action endpoint driven by alarm data
    And an empty context shows an empty state pointing at alarm setup
