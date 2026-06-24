Feature: Opsætning (Setup / administration) module
  As a company administrator
  I want master data, user administration and building elements
  So that I can configure the company

  Background:
    Given I am in the company context "Bolag AB Janus"
    And I have an administrator/company-responsible role

  Scenario: Stamdata is a tabbed master-data editor
    When I open Opsætning > Stamdata
    Then the URL is "/App/company/997/setup/company_data"
    And I see tabs Firmadata, Energipriser, CO₂e faktorer, Frie Nøgletal- og Tags, Frie Stamfelter, Frie målertyper, Generelt, Infoboards and Licens
    And the Firmadata tab shows a read-only company profile form with legal name, address, zip/city, country, phone, email, contact person and status
    And I see an Action menu of company-level actions
    And a "Redigér" action switches the form to edit mode (not exercised in this read-only spec)

  Scenario: Stamdata reflects setup state
    Given the company is under setup
    Then a warning banner indicates the company is under setup

  Scenario: Brugeradministration lists users
    When I open Opsætning > Brugeradministration
    Then the URL is "/App/company/997/user_administration"
    And I see a results grid with columns name, user profile, user id, email, phone, welcome-mail-sent and data access
    And I can search and filter by user profile, welcome-mail-sent and locked status
    And I can toggle a user-activity view
    And I see a "Opret bruger" action (server-rendered form, not exercised here)
    And the grid is sourced from the user-list endpoint

  Scenario: Bygningselementer lists building elements
    When I open Opsætning > Bygningselementer
    Then the URL is "/App/company/997/building_list"
    And I see a results grid with columns designation, building use, city, country, total area, heated area and contact person
    And I can filter by building use, cities, contacts, countries, time zones, building types and energy classes
    And I see a "Opret bygningselement" action (server-rendered form, not exercised here)
    And the grid is sourced from the buildings endpoint with shared building initialization data

  Scenario: Setup surfaces are role-gated
    Then the Opsætning module is only available to admin/company-responsible roles
